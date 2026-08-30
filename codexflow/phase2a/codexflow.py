#!/usr/bin/env python3
from __future__ import annotations

import argparse
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
import time
from contextlib import contextmanager
from datetime import datetime, timezone
from pathlib import Path

SCHEMA = "codexflow.runtime.v1"
TASK_STATUSES = {"todo", "doing", "blocked", "review", "done", "failed", "cancelled"}
AGENT_STATUSES = {"pending", "running", "idle", "blocked", "completed", "failed", "closed"}
GATE_STATUSES = {"pass", "warn", "block", "not_applicable"}
RISKS = {"low", "medium", "high", "critical"}
ID_RE = re.compile(r"^[a-z0-9][a-z0-9_-]{0,63}$")
PROFILE_NAME = "codexflow"


def now_iso() -> str:
    return datetime.now(timezone.utc).replace(microsecond=0).isoformat().replace("+00:00", "Z")


def package_root() -> Path:
    return Path(__file__).resolve().parent


def run_git(args: list[str], cwd: Path | None = None, check: bool = True) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        ["git", *args],
        cwd=str(cwd) if cwd else None,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        check=check,
    )


def repo_root(start: Path | None = None) -> Path:
    start = (start or Path.cwd()).resolve()
    try:
        cp = run_git(["rev-parse", "--show-toplevel"], cwd=start)
    except (FileNotFoundError, subprocess.CalledProcessError) as exc:
        raise SystemExit(f"CodexFlow requires a git repository: {exc}")
    return Path(cp.stdout.strip()).resolve()


def cf_root(repo: Path) -> Path:
    return repo / ".codexflow"


def state_path(repo: Path) -> Path:
    return cf_root(repo) / "state" / "ledger.json"


def events_path(repo: Path) -> Path:
    return cf_root(repo) / "state" / "events.jsonl"


def codex_home() -> Path:
    value = os.environ.get("CODEX_HOME")
    return Path(value).expanduser().resolve() if value else (Path.home() / ".codex").resolve()


def validate_id(value: str, label: str = "id") -> str:
    value = value.strip().lower()
    if not ID_RE.fullmatch(value):
        raise SystemExit(f"invalid {label} '{value}'; use lowercase letters, digits, _ or -, max 64 chars")
    return value


def validate_choice(value: str, allowed: set[str], label: str) -> str:
    if value not in allowed:
        raise SystemExit(f"invalid {label} '{value}'; expected one of: {', '.join(sorted(allowed))}")
    return value


def default_ledger(repo: Path) -> dict:
    return {
        "schema": SCHEMA,
        "repo": str(repo),
        "created_at": now_iso(),
        "updated_at": now_iso(),
        "tasks": {},
        "agents": {},
    }


def atomic_write_json(path: Path, data: dict) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    fd, tmp_name = tempfile.mkstemp(prefix=path.name + ".", suffix=".tmp", dir=path.parent)
    try:
        with os.fdopen(fd, "w", encoding="utf-8", newline="\n") as fh:
            json.dump(data, fh, indent=2, sort_keys=True)
            fh.write("\n")
            fh.flush()
            os.fsync(fh.fileno())
        os.replace(tmp_name, path)
    finally:
        if os.path.exists(tmp_name):
            os.unlink(tmp_name)


@contextmanager
def state_lock(repo: Path, timeout: float = 8.0):
    lock = cf_root(repo) / "state" / ".lock"
    lock.parent.mkdir(parents=True, exist_ok=True)
    deadline = time.monotonic() + timeout
    fd = None
    while fd is None:
        try:
            fd = os.open(str(lock), os.O_CREAT | os.O_EXCL | os.O_WRONLY)
            os.write(fd, f"pid={os.getpid()} at={now_iso()}\n".encode())
        except FileExistsError:
            try:
                if time.time() - lock.stat().st_mtime > 120:
                    lock.unlink()
                    continue
            except FileNotFoundError:
                continue
            if time.monotonic() >= deadline:
                raise SystemExit(f"timed out waiting for CodexFlow state lock: {lock}")
            time.sleep(0.05)
    try:
        yield
    finally:
        try:
            os.close(fd)
        finally:
            try:
                lock.unlink()
            except FileNotFoundError:
                pass


def load_ledger(repo: Path) -> dict:
    path = state_path(repo)
    if not path.exists():
        init_project(repo)
    try:
        data = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as exc:
        raise SystemExit(f"cannot read CodexFlow ledger {path}: {exc}")
    if data.get("schema") != SCHEMA:
        raise SystemExit(f"unsupported CodexFlow ledger schema: {data.get('schema')!r}")
    return data


def save_ledger(repo: Path, data: dict) -> None:
    data["updated_at"] = now_iso()
    atomic_write_json(state_path(repo), data)


def append_event(repo: Path, kind: str, actor: str, message: str, task: str | None = None) -> None:
    event = {
        "ts": now_iso(),
        "kind": kind.strip(),
        "actor": actor.strip(),
        "message": message.strip(),
    }
    if task:
        event["task"] = task
    path = events_path(repo)
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open("a", encoding="utf-8", newline="\n") as fh:
        fh.write(json.dumps(event, separators=(",", ":"), sort_keys=True) + "\n")
        fh.flush()


def init_project(repo: Path) -> None:
    root = cf_root(repo)
    state = root / "state"
    state.mkdir(parents=True, exist_ok=True)
    ignore = root / ".gitignore"
    ignore_text = "# CodexFlow local runtime state\nstate/\ntmp/\n"
    if not ignore.exists():
        ignore.write_text(ignore_text, encoding="utf-8")
    elif "state/" not in ignore.read_text(encoding="utf-8"):
        with ignore.open("a", encoding="utf-8") as fh:
            fh.write("\n" + ignore_text)
    config = root / "config.json"
    if not config.exists():
        atomic_write_json(
            config,
            {
                "schema": "codexflow.project.v1",
                "default_topology": "adaptive",
                "independent_review": True,
                "max_live_agents": 8,
                "pr_automation": False,
                "merge_automation": False,
            },
        )
    if not state_path(repo).exists():
        atomic_write_json(state_path(repo), default_ledger(repo))
    events_path(repo).touch(exist_ok=True)


def render_profile(god_text: str) -> str:
    if '"""' in god_text:
        raise SystemExit("GOD instructions cannot contain TOML triple quotes")
    return (
        f'developer_instructions = """\n{god_text.rstrip()}\n"""\n\n'
        "[features]\n"
        "multi_agent = true\n\n"
        "[agents]\n"
        "max_threads = 8\n"
        "max_depth = 2\n"
    )


def install_runtime(source: Path | None = None) -> dict:
    source = (source or package_root()).resolve()
    home = Path.home() / ".codexflow" / "current"
    bin_dir = Path.home() / ".codexflow" / "bin"
    c_home = codex_home()
    agents_dir = c_home / "agents"
    c_home.mkdir(parents=True, exist_ok=True)
    agents_dir.mkdir(parents=True, exist_ok=True)
    bin_dir.mkdir(parents=True, exist_ok=True)

    if source != home:
        if home.exists():
            shutil.rmtree(home)
        shutil.copytree(source, home, ignore=shutil.ignore_patterns("__pycache__", "*.pyc"))

    god = (home / "prompts" / "GOD.md").read_text(encoding="utf-8")
    profile = c_home / f"{PROFILE_NAME}.config.toml"
    profile.write_text(render_profile(god), encoding="utf-8", newline="\n")

    installed_roles = []
    for role in sorted((home / "roles").glob("trinity_*.toml")):
        target = agents_dir / role.name
        shutil.copy2(role, target)
        installed_roles.append(str(target))

    cmd = bin_dir / "codexflow.cmd"
    cmd.write_text(
        "@echo off\r\n"
        "setlocal\r\n"
        "where py >nul 2>nul && (py -3 \"%USERPROFILE%\\.codexflow\\current\\codexflow.py\" %* & exit /b %ERRORLEVEL%)\r\n"
        "python \"%USERPROFILE%\\.codexflow\\current\\codexflow.py\" %*\r\n",
        encoding="utf-8",
        newline="",
    )
    sh = bin_dir / "codexflow"
    sh.write_text(
        "#!/usr/bin/env sh\nexec python3 \"$HOME/.codexflow/current/codexflow.py\" \"$@\"\n",
        encoding="utf-8",
    )
    try:
        sh.chmod(0o755)
    except OSError:
        pass

    return {
        "runtime": str(home),
        "bin": str(bin_dir),
        "profile": str(profile),
        "roles": installed_roles,
    }


def find_codex() -> str:
    exe = shutil.which("codex")
    if not exe:
        raise SystemExit("codex executable not found on PATH")
    return exe


def launch_codex(repo: Path, extra: list[str]) -> int:
    init_project(repo)
    exe = find_codex()
    env = os.environ.copy()
    env["CODEXFLOW_REPO"] = str(repo)
    env["CODEXFLOW_STATE_DIR"] = str(cf_root(repo) / "state")
    installed_god = Path.home() / ".codexflow" / "current" / "prompts" / "GOD.md"
    god_path = installed_god if installed_god.exists() else package_root() / "prompts" / "GOD.md"
    god = god_path.read_text(encoding="utf-8")
    # Runtime precedence prevents a project-level developer_instructions setting
    # from accidentally disabling GOD mode while still leaving all other project
    # configuration authoritative. json.dumps emits a TOML-compatible quoted string.
    god_override = "developer_instructions=" + json.dumps(god)
    cmd = [
        exe,
        "--profile",
        PROFILE_NAME,
        "--enable",
        "multi_agent",
        "-c",
        god_override,
        *extra,
    ]
    return subprocess.call(cmd, cwd=str(repo), env=env)


def task_create(repo: Path, args: argparse.Namespace) -> None:
    task_id = validate_id(args.id, "task id")
    risk = validate_choice(args.risk, RISKS, "risk")
    with state_lock(repo):
        data = load_ledger(repo)
        if task_id in data["tasks"]:
            raise SystemExit(f"task already exists: {task_id}")
        ts = now_iso()
        data["tasks"][task_id] = {
            "title": args.title.strip(),
            "status": "todo",
            "risk": risk,
            "assignee": args.assignee,
            "depends_on": [validate_id(x, "dependency") for x in (args.depends_on or [])],
            "gates": {},
            "handoffs": [],
            "created_at": ts,
            "updated_at": ts,
        }
        save_ledger(repo, data)
        append_event(repo, "task.created", "god", args.title, task_id)
    print(task_id)


def task_set(repo: Path, args: argparse.Namespace) -> None:
    task_id = validate_id(args.id, "task id")
    with state_lock(repo):
        data = load_ledger(repo)
        task = data["tasks"].get(task_id)
        if not task:
            raise SystemExit(f"unknown task: {task_id}")
        if args.status:
            task["status"] = validate_choice(args.status, TASK_STATUSES, "task status")
        if args.assignee is not None:
            task["assignee"] = args.assignee or None
        if args.risk:
            task["risk"] = validate_choice(args.risk, RISKS, "risk")
        task["updated_at"] = now_iso()
        save_ledger(repo, data)
        append_event(repo, "task.updated", "god", f"status={task['status']} assignee={task.get('assignee')}", task_id)
    print(json.dumps(task, indent=2, sort_keys=True))


def gate_set(repo: Path, args: argparse.Namespace) -> None:
    task_id = validate_id(args.task, "task id")
    gate_name = validate_id(args.name, "gate name")
    status = validate_choice(args.status, GATE_STATUSES, "gate status")
    risk = validate_choice(args.risk, RISKS, "risk")
    with state_lock(repo):
        data = load_ledger(repo)
        task = data["tasks"].get(task_id)
        if not task:
            raise SystemExit(f"unknown task: {task_id}")
        gate = {
            "status": status,
            "risk": risk,
            "reviewer": args.reviewer,
            "finding": args.finding,
            "updated_at": now_iso(),
        }
        task.setdefault("gates", {})[gate_name] = gate
        task["updated_at"] = now_iso()
        save_ledger(repo, data)
        append_event(repo, "gate.updated", args.reviewer or "god", f"{gate_name}={status}", task_id)
    print(json.dumps(gate, indent=2, sort_keys=True))


def agent_set(repo: Path, args: argparse.Namespace) -> None:
    name = args.name.strip()
    if not name:
        raise SystemExit("agent name is required")
    status = validate_choice(args.status, AGENT_STATUSES, "agent status")
    if args.task:
        validate_id(args.task, "task id")
    with state_lock(repo):
        data = load_ledger(repo)
        data["agents"][name] = {
            "role": args.role.strip(),
            "status": status,
            "task": args.task,
            "updated_at": now_iso(),
        }
        save_ledger(repo, data)
        append_event(repo, "agent.updated", name, f"role={args.role} status={status}", args.task)
    print(name)


def handoff_add(repo: Path, args: argparse.Namespace) -> None:
    task_id = validate_id(args.task, "task id")
    refs = args.ref or []
    with state_lock(repo):
        data = load_ledger(repo)
        task = data["tasks"].get(task_id)
        if not task:
            raise SystemExit(f"unknown task: {task_id}")
        handoff = {
            "from": args.from_actor.strip(),
            "to": args.to_actor.strip(),
            "summary": args.summary.strip(),
            "refs": refs,
            "at": now_iso(),
        }
        task.setdefault("handoffs", []).append(handoff)
        task["updated_at"] = now_iso()
        save_ledger(repo, data)
        append_event(repo, "handoff", handoff["from"], f"to={handoff['to']} {handoff['summary']}", task_id)
    print(json.dumps(handoff, indent=2, sort_keys=True))


def snapshot(repo: Path) -> None:
    data = load_ledger(repo)
    blocks = []
    for task_id, task in data["tasks"].items():
        for name, gate in task.get("gates", {}).items():
            if gate.get("status") == "block":
                blocks.append({"task": task_id, "gate": name, **gate})
    result = {
        "schema": data["schema"],
        "repo": data["repo"],
        "updated_at": data["updated_at"],
        "task_counts": {s: sum(1 for t in data["tasks"].values() if t.get("status") == s) for s in sorted(TASK_STATUSES)},
        "live_agents": {k: v for k, v in data["agents"].items() if v.get("status") not in {"completed", "failed", "closed"}},
        "blocking_gates": blocks,
        "tasks": data["tasks"],
    }
    print(json.dumps(result, indent=2, sort_keys=True))


def doctor(repo: Path | None) -> int:
    rows: list[tuple[str, bool, str]] = []
    exe = shutil.which("codex")
    rows.append(("codex", bool(exe), exe or "not found"))
    py_ok = sys.version_info >= (3, 10)
    rows.append(("python", py_ok, sys.version.split()[0]))
    git = shutil.which("git")
    rows.append(("git", bool(git), git or "not found"))
    profile = codex_home() / f"{PROFILE_NAME}.config.toml"
    rows.append(("profile", profile.exists(), str(profile)))
    roles = list((codex_home() / "agents").glob("trinity_*.toml")) if (codex_home() / "agents").exists() else []
    rows.append(("roles", len(roles) >= 5, f"{len(roles)} installed"))
    if repo:
        rows.append(("project", cf_root(repo).exists(), str(cf_root(repo))))
        rows.append(("ledger", state_path(repo).exists(), str(state_path(repo))))
    for name, ok, detail in rows:
        print(f"{'PASS' if ok else 'FAIL':4}  {name:10} {detail}")
    return 0 if all(ok for _, ok, _ in rows) else 1


def build_parser() -> argparse.ArgumentParser:
    p = argparse.ArgumentParser(prog="codexflow", description="CodexFlow native multi-agent control-plane bootstrap")
    p.add_argument("--repo", help="git repository root; defaults to current repository")
    sub = p.add_subparsers(dest="command")

    sub.add_parser("init", help="initialize .codexflow runtime state in the current repository")
    i = sub.add_parser("install", help="install CodexFlow profile, roles and launcher into the user account")
    i.add_argument("--source", help="package source directory")
    l = sub.add_parser("launch", help="launch Codex as the CodexFlow GOD session")
    l.add_argument("codex_args", nargs=argparse.REMAINDER)
    sub.add_parser("doctor", help="verify CodexFlow prerequisites and installation")
    sub.add_parser("snapshot", help="print the current durable task/agent state")

    task = sub.add_parser("task", help="task ledger operations")
    task_sub = task.add_subparsers(dest="task_command", required=True)
    tc = task_sub.add_parser("create")
    tc.add_argument("--id", required=True)
    tc.add_argument("--title", required=True)
    tc.add_argument("--risk", default="medium")
    tc.add_argument("--assignee")
    tc.add_argument("--depends-on", action="append")
    ts = task_sub.add_parser("set")
    ts.add_argument("--id", required=True)
    ts.add_argument("--status", choices=sorted(TASK_STATUSES))
    ts.add_argument("--risk", choices=sorted(RISKS))
    ts.add_argument("--assignee")

    gate = sub.add_parser("gate", help="department/review gate operations")
    gate_sub = gate.add_subparsers(dest="gate_command", required=True)
    gs = gate_sub.add_parser("set")
    gs.add_argument("--task", required=True)
    gs.add_argument("--name", required=True)
    gs.add_argument("--status", required=True, choices=sorted(GATE_STATUSES))
    gs.add_argument("--risk", required=True, choices=sorted(RISKS))
    gs.add_argument("--reviewer")
    gs.add_argument("--finding")

    agent = sub.add_parser("agent", help="mirror native agent lifecycle into durable state")
    agent_sub = agent.add_subparsers(dest="agent_command", required=True)
    aset = agent_sub.add_parser("set")
    aset.add_argument("--name", required=True)
    aset.add_argument("--role", required=True)
    aset.add_argument("--status", required=True, choices=sorted(AGENT_STATUSES))
    aset.add_argument("--task")

    handoff = sub.add_parser("handoff", help="record concise agent handoffs")
    handoff_sub = handoff.add_subparsers(dest="handoff_command", required=True)
    ha = handoff_sub.add_parser("add")
    ha.add_argument("--task", required=True)
    ha.add_argument("--from", dest="from_actor", required=True)
    ha.add_argument("--to", dest="to_actor", required=True)
    ha.add_argument("--summary", required=True)
    ha.add_argument("--ref", action="append")

    event = sub.add_parser("event", help="append an audit event")
    event_sub = event.add_subparsers(dest="event_command", required=True)
    ea = event_sub.add_parser("add")
    ea.add_argument("--kind", required=True)
    ea.add_argument("--actor", required=True)
    ea.add_argument("--message", required=True)
    ea.add_argument("--task")
    return p


def main(argv: list[str] | None = None) -> int:
    args = build_parser().parse_args(argv)
    if not args.command:
        args.command = "launch"
        args.codex_args = []

    if args.command == "install":
        result = install_runtime(Path(args.source) if args.source else None)
        print(json.dumps(result, indent=2))
        return 0

    repo = Path(args.repo).resolve() if args.repo else repo_root()

    if args.command == "init":
        init_project(repo)
        print(cf_root(repo))
        return 0
    if args.command == "launch":
        extra = list(args.codex_args or [])
        if extra and extra[0] == "--":
            extra = extra[1:]
        return launch_codex(repo, extra)
    if args.command == "doctor":
        return doctor(repo)
    if args.command == "snapshot":
        snapshot(repo)
        return 0
    if args.command == "task" and args.task_command == "create":
        task_create(repo, args)
        return 0
    if args.command == "task" and args.task_command == "set":
        task_set(repo, args)
        return 0
    if args.command == "gate" and args.gate_command == "set":
        gate_set(repo, args)
        return 0
    if args.command == "agent" and args.agent_command == "set":
        agent_set(repo, args)
        return 0
    if args.command == "handoff" and args.handoff_command == "add":
        handoff_add(repo, args)
        return 0
    if args.command == "event" and args.event_command == "add":
        task = validate_id(args.task, "task id") if args.task else None
        with state_lock(repo):
            append_event(repo, args.kind, args.actor, args.message, task)
        return 0
    raise SystemExit("unsupported command")


if __name__ == "__main__":
    raise SystemExit(main())
