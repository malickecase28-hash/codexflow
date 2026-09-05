use super::sibling_executable;
use anyhow::Context;
use anyhow::Result;
use anyhow::bail;
use clap::Args;
use clap::Subcommand;
use std::ffi::OsString;
use std::path::Path;
use std::process::Command;

#[derive(Debug, Args)]
pub(super) struct EventArgs {
    #[arg(long)]
    pub(super) project: Option<String>,
    #[command(subcommand)]
    command: EventCommand,
}

#[derive(Debug, Subcommand)]
enum EventCommand {
    Run {
        #[arg(long, default_value = "127.0.0.1:0")]
        bind: String,
    },
    Publish {
        #[arg(long)]
        kind: String,
        #[arg(long)]
        key: Option<String>,
        #[arg(long)]
        dedupe_key: Option<String>,
        #[arg(long, default_value = "{}")]
        payload: String,
    },
    Await {
        #[arg(long)]
        id: String,
        #[arg(long)]
        owner: String,
        #[arg(long = "topic", required = true)]
        topics: Vec<String>,
        #[arg(long)]
        key: Option<String>,
        #[arg(long, default_value_t = 0)]
        after_seq: u64,
        #[arg(long)]
        timeout_at: Option<String>,
    },
    Inbox {
        #[arg(long)]
        owner: String,
        #[arg(long)]
        clear: bool,
    },
    Status,
}

pub(super) fn handle(project_root: &Path, args: EventArgs) -> Result<()> {
    let executable = sibling_executable("codexflow-supervisor")?;
    let status = Command::new(&executable)
        .args(command_args(project_root, &args.command))
        .status()
        .with_context(|| format!("run {}", executable.display()))?;
    if !status.success() {
        bail!("codexflow-supervisor exited with {status}");
    }
    Ok(())
}

fn command_args(project_root: &Path, command: &EventCommand) -> Vec<OsString> {
    let mut args = Vec::new();
    match command {
        EventCommand::Run { bind } => {
            args.extend([OsString::from("run"), OsString::from("--project-root")]);
            args.push(project_root.as_os_str().to_owned());
            args.extend([OsString::from("--bind"), OsString::from(bind)]);
        }
        EventCommand::Publish {
            kind,
            key,
            dedupe_key,
            payload,
        } => {
            args.extend([OsString::from("publish"), OsString::from("--project-root")]);
            args.push(project_root.as_os_str().to_owned());
            args.extend([OsString::from("--kind"), OsString::from(kind)]);
            if let Some(key) = key {
                args.extend([OsString::from("--key"), OsString::from(key)]);
            }
            if let Some(dedupe_key) = dedupe_key {
                args.extend([OsString::from("--dedupe-key"), OsString::from(dedupe_key)]);
            }
            args.extend([OsString::from("--payload"), OsString::from(payload)]);
        }
        EventCommand::Await {
            id,
            owner,
            topics,
            key,
            after_seq,
            timeout_at,
        } => {
            args.extend([OsString::from("await"), OsString::from("--project-root")]);
            args.push(project_root.as_os_str().to_owned());
            args.extend([OsString::from("--id"), OsString::from(id)]);
            args.extend([OsString::from("--owner"), OsString::from(owner)]);
            for topic in topics {
                args.extend([OsString::from("--topic"), OsString::from(topic)]);
            }
            if let Some(key) = key {
                args.extend([OsString::from("--key"), OsString::from(key)]);
            }
            args.extend([
                OsString::from("--after-seq"),
                OsString::from(after_seq.to_string()),
            ]);
            if let Some(timeout_at) = timeout_at {
                args.extend([OsString::from("--timeout-at"), OsString::from(timeout_at)]);
            }
        }
        EventCommand::Inbox { owner, clear } => {
            args.extend([OsString::from("inbox"), OsString::from("--project-root")]);
            args.push(project_root.as_os_str().to_owned());
            args.extend([OsString::from("--owner"), OsString::from(owner)]);
            if *clear {
                args.push(OsString::from("--clear"));
            }
        }
        EventCommand::Status => {
            args.extend([OsString::from("status"), OsString::from("--project-root")]);
            args.push(project_root.as_os_str().to_owned());
        }
    }
    args
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn publish_args_forward_dedupe_key() {
        let rendered = command_args(
            Path::new("/repo"),
            &EventCommand::Publish {
                kind: "build.completed".to_string(),
                key: Some("job-1".to_string()),
                dedupe_key: Some("build:job-1".to_string()),
                payload: "{}".to_string(),
            },
        )
        .into_iter()
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect::<Vec<_>>();
        assert!(
            rendered
                .windows(2)
                .any(|pair| pair[0] == "--dedupe-key" && pair[1] == "build:job-1")
        );
    }

    #[test]
    fn await_args_forward_timeout_deadline() {
        let rendered = command_args(
            Path::new("/repo"),
            &EventCommand::Await {
                id: "wait-1".to_string(),
                owner: "god".to_string(),
                topics: vec!["build.completed".to_string()],
                key: None,
                after_seq: 7,
                timeout_at: Some("2030-01-01T00:00:00Z".to_string()),
            },
        )
        .into_iter()
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect::<Vec<_>>();
        assert!(
            rendered
                .windows(2)
                .any(|pair| pair[0] == "--timeout-at" && pair[1] == "2030-01-01T00:00:00Z")
        );
    }
}
