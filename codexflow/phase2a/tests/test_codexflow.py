import importlib.util
import json
import subprocess
import tempfile
import unittest
from argparse import Namespace
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location("codexflow", ROOT / "codexflow.py")
cf = importlib.util.module_from_spec(SPEC)
assert SPEC.loader
SPEC.loader.exec_module(cf)


class CodexFlowTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.repo = Path(self.tmp.name) / "repo"
        self.repo.mkdir()
        subprocess.run(["git", "init", "-q"], cwd=self.repo, check=True)
        cf.init_project(self.repo)

    def tearDown(self):
        self.tmp.cleanup()

    def test_init_is_idempotent_and_runtime_state_is_ignored(self):
        cf.init_project(self.repo)
        ledger = json.loads(cf.state_path(self.repo).read_text(encoding="utf-8"))
        self.assertEqual(ledger["schema"], cf.SCHEMA)
        ignore = (cf.cf_root(self.repo) / ".gitignore").read_text(encoding="utf-8")
        self.assertIn("state/", ignore)

    def test_task_gate_agent_handoff_lifecycle(self):
        cf.task_create(self.repo, Namespace(id="reconnect_fix", title="Fix reconnect semantics", risk="high", assignee=None, depends_on=None))
        cf.task_set(self.repo, Namespace(id="reconnect_fix", status="doing", assignee="worker_impl", risk=None))
        cf.agent_set(self.repo, Namespace(name="/root/worker_impl", role="trinity_worker", status="running", task="reconnect_fix"))
        cf.gate_set(self.repo, Namespace(task="reconnect_fix", name="independent_review", status="block", risk="high", reviewer="reviewer_1", finding="review pending"))
        cf.handoff_add(self.repo, Namespace(task="reconnect_fix", from_actor="worker_impl", to_actor="reviewer_1", summary="Implementation ready for review", ref=["src/reconnect.rs"]))
        data = cf.load_ledger(self.repo)
        task = data["tasks"]["reconnect_fix"]
        self.assertEqual(task["status"], "doing")
        self.assertEqual(task["assignee"], "worker_impl")
        self.assertEqual(task["gates"]["independent_review"]["status"], "block")
        self.assertEqual(task["handoffs"][0]["refs"], ["src/reconnect.rs"])
        self.assertEqual(data["agents"]["/root/worker_impl"]["role"], "trinity_worker")

    def test_invalid_ids_rejected(self):
        with self.assertRaises(SystemExit):
            cf.validate_id("Bad ID", "task id")


    def test_stale_lock_is_recovered(self):
        lock = cf.cf_root(self.repo) / "state" / ".lock"
        lock.write_text("stale", encoding="utf-8")
        import os, time
        old = time.time() - 300
        os.utime(lock, (old, old))
        with cf.state_lock(self.repo):
            self.assertTrue(lock.exists())
        self.assertFalse(lock.exists())

    def test_profile_enables_multi_agent_and_contains_god_contract(self):
        text = cf.render_profile("GOD test instructions")
        self.assertIn("developer_instructions", text)
        self.assertIn("multi_agent = true", text)
        self.assertIn("max_threads = 8", text)

    def test_atomic_json_replaces_whole_file(self):
        path = cf.state_path(self.repo)
        cf.atomic_write_json(path, {"schema": cf.SCHEMA, "repo": str(self.repo), "created_at": "x", "updated_at": "x", "tasks": {"a": {}}, "agents": {}})
        data = json.loads(path.read_text(encoding="utf-8"))
        self.assertEqual(list(data["tasks"]), ["a"])


if __name__ == "__main__":
    unittest.main()
