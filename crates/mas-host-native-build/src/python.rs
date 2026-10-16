use super::{run, Result};
use std::path::Path;
use std::process::Command;

/// Python is discovered only when a Python agent is requested. The generated
/// zipapp snapshots application and SDK source, rather than importing the live
/// project on subsequent executions.
pub(super) fn build(
    project: &Path,
    source: &Path,
    executable: &Path,
    sdk_root: &Path,
    handler: &str,
) -> Result<()> {
    run(Command::new("python3")
        .arg("-I")
        .arg("-c")
        .arg(include_str!("python_zipapp.py"))
        .arg(project)
        .arg(source)
        .arg(executable)
        .arg(sdk_root.join("sdk/python/mas_host"))
        .arg(handler))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::build;
    use serde_json::json;
    use std::fs;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static NEXT: AtomicUsize = AtomicUsize::new(0);

    struct Fixture {
        root: PathBuf,
        project: PathBuf,
        sdk: PathBuf,
    }

    impl Fixture {
        fn new() -> Self {
            let root = std::env::temp_dir().join(format!(
                "mas-python-build-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir_all(&root).unwrap();
            let project = root.join("application");
            let sdk = root.join("distribution");
            fs::create_dir_all(&project).unwrap();
            fs::create_dir_all(sdk.join("sdk/python/mas_host")).unwrap();
            fs::write(
                sdk.join("sdk/python/mas_host/__init__.py"),
                "def serve_module(handler_name, module_name):\n    import importlib, json\n    print(json.dumps(getattr(importlib.import_module(module_name), handler_name)(None)))\n",
            )
            .unwrap();
            Self { root, project, sdk }
        }

        fn write(&self, path: &str, content: &str) {
            let path = self.project.join(path);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, content).unwrap();
        }

        fn build(&self, source: &str) -> Result<PathBuf> {
            let built = build(
                &self.project,
                &self.project.join(".mas/build"),
                &self.sdk,
                &json!([{"language":"python","name":"test","source":source,"handler":"run"}]),
            )?;
            Ok(PathBuf::from(built[0]["executable"].as_str().unwrap()))
        }

        fn execute(&self, worker: &Path) -> String {
            let output = Command::new(worker)
                .current_dir(&self.root)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            String::from_utf8(output.stdout).unwrap().trim().to_owned()
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    #[test]
    fn rebuilds_are_deterministic_and_helper_changes_change_worker_bytes() {
        let fixture = Fixture::new();
        fixture.write(
            "agent.py",
            "import helper\ndef run(ctx): return helper.VALUE\n",
        );
        fixture.write("helper.py", "VALUE = 7\n");
        let worker = fixture.build("agent.py").unwrap();
        let first = fs::read(&worker).unwrap();
        fixture.build("agent.py").unwrap();
        assert_eq!(first, fs::read(&worker).unwrap());
        fixture.write("helper.py", "VALUE = 8\n");
        fixture.build("agent.py").unwrap();
        assert_ne!(first, fs::read(&worker).unwrap());
        assert_eq!(fixture.execute(&worker), "8");
    }

    #[test]
    fn snapshots_regular_packages_helpers_and_sdk_before_source_removal() {
        let fixture = Fixture::new();
        fixture.write("agents/__init__.py", "");
        fixture.write(
            "agents/agent.py",
            "from .helper import VALUE\ndef run(ctx): return VALUE\n",
        );
        fixture.write("agents/helper.py", "VALUE = 9007199254740993\n");
        let worker = fixture.build("agents/agent.py").unwrap();
        fixture.write("agents/helper.py", "VALUE = 0\n");
        fs::remove_dir_all(fixture.project.join("agents")).unwrap();
        fs::remove_dir_all(&fixture.sdk).unwrap();
        assert_eq!(fixture.execute(&worker), "9007199254740993");
    }

    #[test]
    fn sdk_source_changes_alter_the_frozen_worker() {
        let fixture = Fixture::new();
        fixture.write("agent.py", "def run(ctx): return 1\n");
        let worker = fixture.build("agent.py").unwrap();
        let first = fs::read(&worker).unwrap();
        fs::write(
            fixture.sdk.join("sdk/python/mas_host/helper.py"),
            "VERSION = 2\n",
        )
        .unwrap();
        fixture.build("agent.py").unwrap();
        assert_ne!(first, fs::read(&worker).unwrap());
    }

    #[test]
    fn preserves_namespace_package_imports() {
        let fixture = Fixture::new();
        fixture.write(
            "agents/nested/agent.py",
            "from helpers.constants import VALUE\ndef run(ctx): return VALUE\n",
        );
        fixture.write("helpers/constants.py", "VALUE = 12\n");
        let worker = fixture.build("agents/nested/agent.py").unwrap();
        assert_eq!(fixture.execute(&worker), "12");
    }

    #[test]
    fn accepts_package_initialisers_as_entry_modules() {
        let fixture = Fixture::new();
        fixture.write("agents/__init__.py", "def run(ctx): return 4\n");
        let worker = fixture.build("agents/__init__.py").unwrap();
        assert_eq!(fixture.execute(&worker), "4");
    }

    #[test]
    fn excludes_generated_and_hidden_directories_and_never_imports_at_build() {
        let fixture = Fixture::new();
        fixture.write(
            "agent.py",
            "raise RuntimeError('must not import while building')\ndef run(ctx): return 1\n",
        );
        for directory in [
            ".git",
            ".venv",
            ".mas",
            "venv",
            "build",
            "dist",
            "node_modules",
            "target",
            "__pycache__",
        ] {
            fixture.write(
                &format!("{directory}/invalid.py"),
                "this is not valid python !!",
            );
        }
        assert!(fixture.build("agent.py").is_ok());
    }

    #[test]
    fn rejects_syntax_errors_and_non_importable_entry_modules() {
        let fixture = Fixture::new();
        fixture.write("agent.py", "def run(ctx): return 1\n");
        fixture.write("helper.py", "this is not valid python !!");
        assert!(fixture.build("agent.py").unwrap_err().contains("helper.py"));
        fs::remove_file(fixture.project.join("helper.py")).unwrap();
        fixture.write("not-a-module.py", "def run(ctx): return 1\n");
        assert!(fixture
            .build("not-a-module.py")
            .unwrap_err()
            .contains("importable"));
        fixture.write("agent.txt", "def run(ctx): return 1\n");
        assert!(fixture.build("agent.txt").unwrap_err().contains(".py"));
    }

    #[test]
    fn rejects_launcher_and_sdk_name_conflicts() {
        for reserved in ["__main__.py", "mas_host.py", "mas_host/__init__.py"] {
            let fixture = Fixture::new();
            fixture.write("agent.py", "def run(ctx): return 1\n");
            fixture.write(reserved, "");
            assert!(fixture.build("agent.py").unwrap_err().contains("reserved"));
        }
    }

    #[test]
    fn rejects_oversized_snapshots_before_compiling_or_publishing() {
        let fixture = Fixture::new();
        fixture.write("agent.py", "def run(ctx): return 1\n");
        fixture.write("helper.py", &"#".repeat(16 * 1024 * 1024));
        let error = fixture.build("agent.py").unwrap_err();
        assert!(error.contains("16 MiB"));
        assert!(!fixture.project.join(".mas/build/native-0/worker").exists());
    }

    #[cfg(unix)]
    #[test]
    fn rejects_project_symlinks_including_internal_ones() {
        use std::os::unix::fs::symlink;
        for target in ["agent.py", "/etc/passwd"] {
            let fixture = Fixture::new();
            fixture.write("agent.py", "def run(ctx): return 1\n");
            symlink(target, fixture.project.join("helper.py")).unwrap();
            assert!(fixture.build("agent.py").unwrap_err().contains("symlink"));
        }
    }
}
