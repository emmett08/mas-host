//! Developer and administrative commands for the local native host.
mod build;
mod config;
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

const HELP: &str = "mas-host: native applications on the MAS protocol\n\n\
  build   --project DIR [--sdk-root DIR] [--agents FILE] [--connections FILE]\n\
  run     --project DIR --agent NAME --input FILE --state DIR --run ID [--sdk-root DIR] [--agents FILE] [--connections FILE]\n\
  run     --bundle FILE --agent NAME --input FILE --state DIR --run ID\n\
  inspect --state DIR --run ID\n\
  cancel  --state DIR --run ID\n\
  resolve --state DIR --run ID --effect ID --output FILE --evidence FILE\n\n\
Run builds automatically when --project is supplied. Reuse --run to resume.\n\
The state directory belongs to one local operator. No Python runtime is required.\n";

fn options(args: &[String], allowed: &[&str]) -> Result<BTreeMap<String, String>, String> {
    if args.len() % 2 != 0 {
        return Err("options require --name VALUE pairs".into());
    }
    let mut result = BTreeMap::new();
    for pair in args.chunks_exact(2) {
        if !allowed.contains(&pair[0].as_str()) {
            return Err(format!("unknown option {}", pair[0]));
        }
        if result.insert(pair[0].clone(), pair[1].clone()).is_some() {
            return Err(format!("duplicate option {}", pair[0]));
        }
    }
    Ok(result)
}

fn require<'a>(args: &'a BTreeMap<String, String>, key: &str) -> Result<&'a str, String> {
    args.get(key)
        .map(String::as_str)
        .ok_or_else(|| format!("missing {key}"))
}

fn sdk_root(args: &BTreeMap<String, String>) -> PathBuf {
    args.get("--sdk-root")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("MAS_HOST_SDK_ROOT").map(PathBuf::from))
        .unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("../.."))
}

fn execute(args: &[String]) -> Result<Value, String> {
    let command = args.first().ok_or("missing command (try --help)")?;
    match command.as_str() {
        "build" => {
            let args = options(
                &args[1..],
                &["--project", "--sdk-root", "--agents", "--connections"],
            )?;
            let file = build::build_configured(
                Path::new(require(&args, "--project")?),
                &sdk_root(&args),
                args.get("--agents").map(Path::new),
                args.get("--connections").map(Path::new),
            )?;
            Ok(json!({"status":"built","bundle":file}))
        }
        "run" => {
            let args = options(
                &args[1..],
                &[
                    "--project",
                    "--sdk-root",
                    "--bundle",
                    "--agent",
                    "--input",
                    "--state",
                    "--run",
                    "--agents",
                    "--connections",
                ],
            )?;
            let input = config::read_json(Path::new(require(&args, "--input")?))?;
            if !input.is_object() {
                return Err("application input must be a JSON object".into());
            }
            let agent = require(&args, "--agent")?;
            let run = require(&args, "--run")?;
            let state = Path::new(require(&args, "--state")?);
            let bundle_path = match (args.get("--bundle"), args.get("--project")) {
                (Some(file), None)
                    if !args.contains_key("--agents") && !args.contains_key("--connections") =>
                {
                    PathBuf::from(file)
                }
                (None, Some(project)) => build::build_configured(
                    Path::new(project),
                    &sdk_root(&args),
                    args.get("--agents").map(Path::new),
                    args.get("--connections").map(Path::new),
                )?,
                _ => return Err("provide exactly one of --project and --bundle".into()),
            };
            let bundle = config::read_json(&bundle_path)?;
            let engine = mas_host_core::Engine::open(&bundle, state, run)?;
            let output = engine.run(agent, &input)?;
            Ok(json!({"status":"completed","run_id":run,"output":output}))
        }
        "inspect" => {
            let args = options(&args[1..], &["--state", "--run"])?;
            mas_host_core::inspect(
                Path::new(require(&args, "--state")?),
                require(&args, "--run")?,
            )
        }
        "cancel" => {
            let args = options(&args[1..], &["--state", "--run"])?;
            mas_host_core::cancel(
                Path::new(require(&args, "--state")?),
                require(&args, "--run")?,
            )
        }
        "resolve" => {
            let args = options(
                &args[1..],
                &["--state", "--run", "--effect", "--output", "--evidence"],
            )?;
            let output = config::read_json(Path::new(require(&args, "--output")?))?;
            let evidence = config::read_json(Path::new(require(&args, "--evidence")?))?;
            mas_host_core::resolve(
                Path::new(require(&args, "--state")?),
                require(&args, "--run")?,
                require(&args, "--effect")?,
                &output,
                &evidence,
            )
        }
        _ => Err(format!("unknown command {command} (try --help)")),
    }
}

fn main() {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.first().is_some_and(|s| s == "__supervise") {
        if mas_protocol_codec::worker_supervisor::supervise(&args[1..]).is_err() {
            std::process::exit(1);
        }
        return;
    }
    if args == ["__builtin"] {
        if mas_host_runtime::serve().is_err() {
            std::process::exit(1);
        }
        return;
    }
    if args.is_empty() || args.iter().any(|s| s == "--help" || s == "-h") {
        print!("{HELP}");
        return;
    }
    if args == ["--version"] {
        println!("mas-host {} / mas-native/1", env!("CARGO_PKG_VERSION"));
        return;
    }
    match execute(&args) {
        Ok(result) => println!("{}", serde_json::to_string_pretty(&result).unwrap()),
        Err(error) => {
            eprintln!("{}", json!({"status":"error","error":error}));
            std::process::exit(1);
        }
    }
}
