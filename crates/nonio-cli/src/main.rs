//! The nonio command.
//!   nonio mcp      run as an MCP server over stdio (the agent starts it; D29)
//! Options: --repo <dir> (default: the host's project folder, else the current one)  --port <n> (default 4317)
//!
//! After `packages/server/src/cli.ts` and `runChannel` in `packages/server/src/channel.ts` (MIT, Copyright (c) 2026
//! johnz4021); see NOTICE. nonio has only the MCP command: the maps are made by the agent's own model.

use std::io::IsTerminal;
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;

use nonio_server::channel::{Channel, ChannelOptions, PORT};
use nonio_server::server::Viewer;
use rmcp::ServiceExt;

/// The viewer as built in `viewer/dist` when this binary was compiled. A build without it still runs; its pages say
/// how to build the viewer.
#[derive(rust_embed::RustEmbed)]
#[folder = "../../viewer/dist"]
#[allow_missing = true]
struct Built;

fn viewer() -> Viewer {
    Viewer::Files(Arc::new(|rel: &str| Built::get(rel).map(|f| f.data)))
}

const HELP: &str = "nonio: interactive maps of how your code works, checked against the code.

  nonio mcp        run as an MCP server over stdio (your agent starts it)

Options: --repo <dir>  --port <n>
Add it to your agent, for example:  claude mcp add --scope user nonio -- nonio mcp";

struct Args {
    command: Option<String>,
    repo: Option<PathBuf>,
    port: u16,
    help: bool,
    version: bool,
}

fn parse_args(argv: impl IntoIterator<Item = String>) -> Result<Args, String> {
    let mut a = Args { command: None, repo: None, port: PORT, help: false, version: false };
    let mut it = argv.into_iter();
    while let Some(x) = it.next() {
        match x.as_str() {
            "-h" | "--help" => a.help = true,
            "-V" | "--version" => a.version = true,
            "--repo" => a.repo = Some(it.next().ok_or("--repo needs a folder")?.into()),
            "--port" => {
                let p = it.next().ok_or("--port needs a number")?;
                a.port = p.parse().map_err(|_| format!("Not a port: {p}"))?;
            }
            _ if x.starts_with('-') => return Err(format!("Unknown option {x}")),
            _ if a.command.is_none() => a.command = Some(x),
            _ => return Err(format!("Unexpected {x}")),
        }
    }
    Ok(a)
}

fn main() -> ExitCode {
    let args = match parse_args(std::env::args().skip(1)) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("{e}\n\n{HELP}");
            return ExitCode::from(2);
        }
    };
    if args.version {
        println!("nonio {}", env!("CARGO_PKG_VERSION"));
        return ExitCode::SUCCESS;
    }
    match args.command.as_deref() {
        Some("mcp") if !args.help => run_mcp(args),
        Some(other) if !args.help => {
            eprintln!("Unknown command {other}.\n\n{HELP}");
            ExitCode::from(2)
        }
        _ => {
            // Started by an MCP client without `mcp` (stdin is a pipe): say so where its logs show it, not on stdout.
            if !args.help && !std::io::stdin().is_terminal() {
                eprintln!("nonio was started with no command. To run it as an MCP server, add `mcp`:\n  nonio mcp");
                return ExitCode::from(1);
            }
            println!("{HELP}");
            ExitCode::SUCCESS
        }
    }
}

fn run_mcp(args: Args) -> ExitCode {
    let rt = match tokio::runtime::Runtime::new() {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("nonio: {e}");
            return ExitCode::from(1);
        }
    };
    rt.block_on(async move {
        // stdout carries the protocol; nothing else may be written there.
        let root = args
            .repo
            .or_else(|| std::env::var_os("CLAUDE_PROJECT_DIR").map(PathBuf::from))
            .or_else(|| std::env::current_dir().ok())
            .unwrap_or_else(|| PathBuf::from("."));
        let mut o = ChannelOptions::new(root);
        o.port = args.port;
        o.viewer = Some(viewer());
        let channel = Channel::new(o);
        let service = match channel.clone().serve(rmcp::transport::stdio()).await {
            Ok(s) => s,
            Err(e) => {
                eprintln!("nonio: the MCP connection failed: {e}");
                return ExitCode::from(1);
            }
        };
        let ct = service.cancellation_token();
        tokio::select! {
            _ = service.waiting() => {}
            _ = stop_signal() => ct.cancel(),
        }
        channel.close().await;
        ExitCode::SUCCESS
    })
}

async fn stop_signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};
        let mut term = signal(SignalKind::terminate()).expect("SIGTERM handler");
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {}
            _ = term.recv() => {}
        }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}
