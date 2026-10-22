//! Developer and administrative commands for the local native host.
mod a2a;
mod build;
mod capabilities;
mod config;
mod lifecycle;
mod privacy_inspect;
#[cfg(feature = "shared-native")]
mod shared_connection;
#[cfg(feature = "shared-native")]
mod shared_native;
#[cfg(feature = "shared-native")]
mod shared_queue;
mod source_quarantine;
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

const HELP: &str = "mas-host: native applications on the MAS protocol\n\n\
  build   --project DIR [--sdk-root DIR] [--agents FILE] [--topology FILE] [--connections FILE] [--capabilities FILE --capability-policy FILE]\n\
  run     --project DIR --agent NAME --input FILE|- [--input-format json|toml|yaml] --state DIR --run ID [--sdk-root DIR] [--agents FILE] [--topology FILE] [--connections FILE]\n\
  run     --bundle FILE --agent NAME --input FILE|- [--input-format json|toml|yaml] --state DIR --run ID\n\
  inspect --state DIR --run ID\n\
  describe --bundle FILE\n\
  govern-lifecycle --state DIR --run ID --governance FILE --tenant ID --principal ID --purpose ID --grant ID --action hold|release-hold|expire|retire [--expires-at-ms N] [--privacy-inspector FILE] [--service-state DIR]\n\
  govern-source-quarantine --governance FILE --tenant ID --principal ID --purpose ID --grant ID --actor NAME --action initialize|list|read|adjudicate|hold|release-hold|expire|retire [--reference ID --after ID --review FILE --expires-at-ms N --privacy-inspector FILE]\n\
  check-profile --profile FILE\n\
  check-capability --declaration FILE\n\
  privacy-inspect --input FILE|- --python ABS --manifest FILE --assets DIR [--worker FILE] [--timeout-ms N] [--memory-mib N] [--include-details true|false]\n\
  serve-a2a --bundle FILE --state DIR --config FILE [--capability-policy FILE]\n\
  observe --state DIR --run ID [--include-payloads true|false]\n\
  shared-native --authority FILE --bundle FILE --input FILE --state DIR --run ID --agent NAME [--reservation-microusd N] (feature shared-native)\n\
  shared-drain --authority FILE --bundle FILE --state DIR --agent NAME --workflow NAME --workflow-identity SHA --turn ID [--after N --page-size N --max-pages N --max-claims N] (feature shared-native)\n\
  cancel  --state DIR --run ID\n\
  resolve --state DIR --run ID --effect ID --output FILE --evidence FILE\n\n\
Run builds automatically when --project is supplied. Reuse --run to resume.\n\
Packaged runs require --capability-policy FILE; project runs may add --capabilities FILE.\n\
Pass --revision-plan FILE to run a frozen assessment/revision workflow.\n\
Governed CLI runs add --governance FILE --tenant ID --principal ID --purpose ID --grant ID [--privacy-inspector FILE].\n\
Input format follows the file extension; stdin defaults to JSON.\n\
The state directory belongs to one local operator. Python agents require Python 3.11+.\n";

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
        "govern-source-quarantine" => source_quarantine::execute(&options(
            &args[1..],
            &[
                "--governance",
                "--tenant",
                "--principal",
                "--purpose",
                "--grant",
                "--actor",
                "--action",
                "--reference",
                "--after",
                "--review",
                "--expires-at-ms",
                "--privacy-inspector",
            ],
        )?),
        "govern-lifecycle" => {
            let args = options(
                &args[1..],
                &[
                    "--state",
                    "--run",
                    "--governance",
                    "--tenant",
                    "--principal",
                    "--purpose",
                    "--grant",
                    "--action",
                    "--expires-at-ms",
                    "--privacy-inspector",
                    "--service-state",
                ],
            )?;
            lifecycle::execute(&args)
        }
        #[cfg(feature = "shared-native")]
        "shared-drain" => {
            let args = options(
                &args[1..],
                &[
                    "--authority",
                    "--bundle",
                    "--state",
                    "--agent",
                    "--reservation-microusd",
                    "--workflow",
                    "--workflow-identity",
                    "--turn",
                    "--after",
                    "--page-size",
                    "--max-pages",
                    "--max-claims",
                ],
            )?;
            shared_native::drain(&args)
        }
        #[cfg(feature = "shared-native")]
        "shared-native" => {
            let args = options(
                &args[1..],
                &[
                    "--authority",
                    "--bundle",
                    "--input",
                    "--state",
                    "--run",
                    "--agent",
                    "--reservation-microusd",
                ],
            )?;
            shared_native::execute(&args)
        }
        "privacy-inspect" => {
            let args = options(
                &args[1..],
                &[
                    "--input",
                    "--python",
                    "--manifest",
                    "--assets",
                    "--worker",
                    "--timeout-ms",
                    "--memory-mib",
                    "--include-details",
                ],
            )?;
            privacy_inspect::execute(&args)
        }
        "check-profile" => {
            let args = options(&args[1..], &["--profile"])?;
            mas_host_core::governance::check_profile(&config::read_json(Path::new(require(
                &args,
                "--profile",
            )?))?)
        }
        "check-capability" => {
            let args = options(&args[1..], &["--declaration"])?;
            capabilities::check(Path::new(require(&args, "--declaration")?))
        }
        "describe" => {
            let args = options(&args[1..], &["--bundle"])?;
            mas_host_core::describe(&config::read_json(Path::new(require(&args, "--bundle")?))?)
        }
        "serve-a2a" => {
            let args = options(
                &args[1..],
                &["--bundle", "--state", "--config", "--capability-policy"],
            )?;
            let bundle = config::read_json(Path::new(require(&args, "--bundle")?))?;
            capabilities::validate_bundle(&bundle, args.get("--capability-policy").map(Path::new))?;
            a2a::serve(
                bundle,
                Path::new(require(&args, "--state")?),
                config::read_json(Path::new(require(&args, "--config")?))?,
            )
        }
        "build" => {
            let args = options(
                &args[1..],
                &[
                    "--project",
                    "--sdk-root",
                    "--agents",
                    "--topology",
                    "--connections",
                    "--capabilities",
                    "--capability-policy",
                ],
            )?;
            let file = build::build_with_capabilities(
                Path::new(require(&args, "--project")?),
                &sdk_root(&args),
                args.get("--agents").map(Path::new),
                args.get("--connections").map(Path::new),
                args.get("--topology").map(Path::new),
                args.get("--capabilities").map(Path::new),
                args.get("--capability-policy").map(Path::new),
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
                    "--topology",
                    "--input-format",
                    "--revision-plan",
                    "--governance",
                    "--tenant",
                    "--principal",
                    "--purpose",
                    "--grant",
                    "--privacy-inspector",
                    "--capabilities",
                    "--capability-policy",
                ],
            )?;
            let input = config::authoring::read_input(
                Path::new(require(&args, "--input")?),
                args.get("--input-format").map(String::as_str),
            )?;
            if !input.is_object() {
                return Err("application input must be an object".into());
            }
            let agent = require(&args, "--agent")?;
            let run = require(&args, "--run")?;
            let state = Path::new(require(&args, "--state")?);
            let bundle_path = match (args.get("--bundle"), args.get("--project")) {
                (Some(file), None)
                    if !args.contains_key("--agents")
                        && !args.contains_key("--connections")
                        && !args.contains_key("--topology") =>
                {
                    PathBuf::from(file)
                }
                (None, Some(project)) => build::build_with_capabilities(
                    Path::new(project),
                    &sdk_root(&args),
                    args.get("--agents").map(Path::new),
                    args.get("--connections").map(Path::new),
                    args.get("--topology").map(Path::new),
                    args.get("--capabilities").map(Path::new),
                    args.get("--capability-policy").map(Path::new),
                )?,
                _ => return Err("provide exactly one of --project and --bundle".into()),
            };
            let bundle = config::read_json(&bundle_path)?;
            if args.contains_key("--bundle") && args.contains_key("--capabilities") {
                return Err("--capabilities requires --project".into());
            }
            let capability_policy = args.get("--capability-policy").map(|path| {
                let path = PathBuf::from(path);
                if path.is_absolute() {
                    path
                } else {
                    args.get("--project")
                        .map(PathBuf::from)
                        .unwrap_or_default()
                        .join(path)
                }
            });
            capabilities::validate_bundle(&bundle, capability_policy.as_deref())?;
            let engine = if let Some(file) = args.get("--governance") {
                let mut session = mas_host_core::governance::GovernedSession::from_value(
                    &config::read_json(Path::new(file))?,
                    require(&args, "--tenant")?,
                    require(&args, "--principal")?,
                    require(&args, "--purpose")?,
                    require(&args, "--grant")?,
                )?;
                if let Some(file) = args.get("--privacy-inspector") {
                    session = privacy_inspect::GovernedInspector::load(Path::new(file))?
                        .attach(session)?;
                }
                mas_host_core::Engine::open_governed(&bundle, state, run, session)?
            } else {
                if [
                    "--tenant",
                    "--principal",
                    "--purpose",
                    "--grant",
                    "--privacy-inspector",
                ]
                .iter()
                .any(|key| args.contains_key(*key))
                {
                    return Err("governed identity options require --governance".into());
                }
                mas_host_core::Engine::open(&bundle, state, run)?
            };
            if let Some(path) = args.get("--revision-plan") {
                let plan = config::read_json(Path::new(path))?;
                if plan["draft_agent"] != agent {
                    return Err("--agent must equal revision plan draft_agent".into());
                }
                let mut result = engine.revise(&plan, &input)?;
                result["status"] = result["revision"]["status"].clone();
                result["run_id"] = json!(run);
                Ok(result)
            } else {
                let output = engine.run(agent, &input)?;
                Ok(json!({"status":"completed","run_id":run,"output":output}))
            }
        }
        "inspect" => {
            let args = options(&args[1..], &["--state", "--run"])?;
            mas_host_core::inspect(
                Path::new(require(&args, "--state")?),
                require(&args, "--run")?,
            )
        }
        "observe" => {
            let args = options(&args[1..], &["--state", "--run", "--include-payloads"])?;
            let include_payloads = match args.get("--include-payloads").map(String::as_str) {
                None | Some("false") => false,
                Some("true") => true,
                _ => return Err("--include-payloads must be true or false".into()),
            };
            mas_host_core::observe(
                Path::new(require(&args, "--state")?),
                require(&args, "--run")?,
                include_payloads,
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
