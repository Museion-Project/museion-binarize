//! Pre-spawn consistency guard for the opt-in local resource bundle.
//! The manifest is unsigned: this is not supply-chain authentication or an
//! atomic fd-exec guarantee. Python repeats inventory/loaded-origin checks.
use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::{Error, ErrorKind, Read},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    time::UNIX_EPOCH,
};

// runtime/1 wire contract. A Python parity test checks its producer whitelist.
pub const SOURCE_FILES: &[&str] = &[
    "scripts/ocr/app_mvp_bridge/__init__.py",
    "scripts/ocr/app_mvp_bridge/__main__.py",
    "scripts/ocr/app_mvp_bridge/sessions.py",
    "scripts/ocr/app_mvp_bridge/save_recovery.py",
    "scripts/ocr/app_mvp_bridge/runtime_contract.py",
    "scripts/ocr/mvp/__init__.py",
    "scripts/ocr/mvp/__main__.py",
    "scripts/ocr/mvp/core.py",
    "scripts/ocr/mvp/local.py",
    "scripts/ocr/mvp/store.py",
    "scripts/ocr/free_local/pipeline.py",
];
const INITIALIZERS: &[&str] = &["scripts/__init__.py", "scripts/ocr/__init__.py"];
const METADATA: &[&str] = &[
    "runtime-manifest.json",
    "resource-build.json",
    "tauri.generated.overlay.json",
];

fn reject(message: &str) -> Error {
    Error::new(ErrorKind::InvalidData, message)
}
fn check(ok: bool, message: &str) -> std::io::Result<()> {
    if ok {
        Ok(())
    } else {
        Err(reject(message))
    }
}
fn no_symlinks(path: &Path) -> std::io::Result<()> {
    for parent in path.ancestors() {
        check(
            !fs::symlink_metadata(parent)?.file_type().is_symlink(),
            "RESOURCE_SYMLINK",
        )?;
    }
    Ok(())
}
fn relative(name: &str) -> std::io::Result<&str> {
    check(
        !name.is_empty()
            && !name.contains('\\')
            && name
                .split('/')
                .all(|p| !p.is_empty() && p != "." && p != "..")
            && !Path::new(name).is_absolute(),
        "RESOURCE_PATH_ESCAPE",
    )?;
    Ok(name)
}
fn string<'a>(value: &'a Value, key: &str) -> std::io::Result<&'a str> {
    value[key]
        .as_str()
        .ok_or_else(|| reject("RESOURCE_MANIFEST_FIELD"))
}
fn object<'a>(value: &'a Value, key: &str) -> std::io::Result<&'a serde_json::Map<String, Value>> {
    value[key]
        .as_object()
        .ok_or_else(|| reject("RESOURCE_IDENTITY_INCOMPLETE"))
}
fn sha(path: &Path) -> std::io::Result<String> {
    let mut file = fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 65536];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}
#[derive(Debug, PartialEq, Eq, Clone)]
struct Stamp {
    len: u64,
    modified_ns: u128,
    device: u64,
    inode: u64,
}
fn stamp(path: &Path) -> std::io::Result<Stamp> {
    let meta = fs::symlink_metadata(path)?;
    check(!meta.file_type().is_symlink(), "RESOURCE_SYMLINK")?;
    #[cfg(unix)]
    let (device, inode) = {
        use std::os::unix::fs::MetadataExt;
        (meta.dev(), meta.ino())
    };
    #[cfg(not(unix))]
    let (device, inode) = (0, 0);
    Ok(Stamp {
        len: meta.len(),
        modified_ns: meta
            .modified()?
            .duration_since(UNIX_EPOCH)
            .map_err(|_| reject("RESOURCE_TIME"))?
            .as_nanos(),
        device,
        inode,
    })
}
#[derive(Debug, Serialize, Clone)]
pub struct LaunchProof {
    pub schema: &'static str,
    pub resource_root: PathBuf,
    pub python: PathBuf,
    pub python_sha256: String,
    pub manifest_sha256: String,
    pub inventory_files: usize,
    pub code_files: usize,
    pub inventory_verified_before_spawn: bool,
    pub preparation_identity_rechecked: bool,
    pub manifest_trusted: bool,
    pub atomic_fd_exec_proven: bool,
    pub distribution_ready: bool,
}
#[derive(Debug)]
struct Verified {
    proof: LaunchProof,
    stamps: BTreeMap<String, Stamp>,
    root_stamp: Stamp,
}
fn scan(root: &Path, dir: &Path, names: &mut BTreeSet<String>) -> std::io::Result<()> {
    for item in fs::read_dir(dir)? {
        let path = item?.path();
        let meta = fs::symlink_metadata(&path)?;
        check(!meta.file_type().is_symlink(), "RESOURCE_SYMLINK")?;
        if meta.is_dir() {
            scan(root, &path, names)?;
        } else {
            check(meta.is_file(), "RESOURCE_NOT_REGULAR")?;
            let name = path
                .strip_prefix(root)
                .map_err(|_| reject("RESOURCE_PATH_ESCAPE"))?
                .to_str()
                .ok_or_else(|| reject("RESOURCE_PATH_ENCODING"))?
                .replace('\\', "/");
            if !METADATA.contains(&name.as_str()) {
                names.insert(name);
            }
        }
    }
    Ok(())
}
fn verify(root: &Path) -> std::io::Result<Verified> {
    check(root.is_absolute(), "RESOURCE_ROOT_ABSOLUTE_REQUIRED")?;
    no_symlinks(root)?;
    let root = root.canonicalize()?;
    let root_before = stamp(&root)?;
    let manifest_path = root.join("runtime-manifest.json");
    no_symlinks(&manifest_path)?;
    let manifest_stamp = stamp(&manifest_path)?;
    let manifest_hash = sha(&manifest_path)?;
    let manifest: Value = serde_json::from_slice(&fs::read(&manifest_path)?)
        .map_err(|_| reject("RESOURCE_MANIFEST_JSON"))?;
    check(
        manifest["schema"] == "museion-local-runtime/1",
        "RESOURCE_MANIFEST_SCHEMA",
    )?;
    check(
        manifest["modes"] == serde_json::json!(["local"]),
        "RESOURCE_LOCAL_MODE_REQUIRED",
    )?;
    let code = object(&manifest, "code_files")?;
    let files = object(&manifest, "runtime_files")?;
    check(
        !code.is_empty() && !files.is_empty(),
        "RESOURCE_IDENTITY_INCOMPLETE",
    )?;
    let expected: BTreeSet<_> = SOURCE_FILES.iter().map(|s| s.to_string()).collect();
    check(
        code.keys().cloned().collect::<BTreeSet<_>>() == expected,
        "RESOURCE_CODE_WHITELIST_INCOMPLETE_OR_EXTRA",
    )?;
    check(
        code.iter().all(|(k, v)| files.get(k) == Some(v)),
        "RESOURCE_CODE_INVENTORY_CONFLICT",
    )?;
    check(
        INITIALIZERS.iter().all(|k| files.contains_key(*k)),
        "RESOURCE_INITIALIZERS_MISSING",
    )?;
    let mut stamps = BTreeMap::new();
    stamps.insert("runtime-manifest.json".to_string(), manifest_stamp);
    for (name, value) in files {
        relative(name)?;
        let hash = value
            .as_str()
            .ok_or_else(|| reject("RESOURCE_HASH_FORMAT"))?;
        check(
            hash.len() == 64
                && hash
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),
            "RESOURCE_HASH_FORMAT",
        )?;
        let namespace = name.split('/').next().unwrap();
        check(
            [
                "scripts",
                "python",
                "native",
                "bin",
                "fonts",
                "tessdata",
                "notices",
                "provenance",
            ]
            .contains(&namespace),
            "RESOURCE_UNDECLARED_NAMESPACE",
        )?;
        if namespace == "scripts" {
            check(
                expected.contains(name) || INITIALIZERS.contains(&name.as_str()),
                "RESOURCE_NONLOCAL_CODE",
            )?;
        }
        let basename = Path::new(name).file_name().unwrap().to_str().unwrap();
        check(
            !name.ends_with(".pth")
                && !basename.starts_with("sitecustomize")
                && !basename.starts_with("usercustomize"),
            "RESOURCE_IMPLICIT_RUNTIME_HOOK",
        )?;
        let path = root.join(name);
        no_symlinks(&path)?;
        check(path.is_file(), "RESOURCE_MISSING")?;
        let before = stamp(&path)?;
        check(sha(&path)? == hash, "RESOURCE_HASH_MISMATCH")?;
        check(stamp(&path)? == before, "RESOURCE_CHANGED_DURING_CHECK")?;
        if INITIALIZERS.contains(&name.as_str()) {
            check(before.len == 0, "RESOURCE_BOOTSTRAP_INITIALIZER_NOT_EMPTY")?;
        }
        stamps.insert(name.clone(), before);
    }
    for key in ["python", "apple_helper", "tesseract", "font"] {
        let path = relative(string(&manifest, key)?)?;
        check(files.contains_key(path), "RESOURCE_CONTRACT_UNHASHED")?;
    }
    let tessdata = relative(string(&manifest, "tessdata")?)?;
    for lang in ["eng", "grc"] {
        check(
            files.contains_key(&format!("{tessdata}/{lang}.traineddata")),
            "RESOURCE_TESSDATA_UNHASHED",
        )?;
    }
    let mut actual = BTreeSet::new();
    scan(&root, &root, &mut actual)?;
    check(
        actual == files.keys().cloned().collect(),
        "RESOURCE_INVENTORY_MISSING_OR_EXTRA",
    )?;
    check(
        stamp(&root)? == root_before
            && stamp(&manifest_path)? == stamps["runtime-manifest.json"]
            && sha(&manifest_path)? == manifest_hash,
        "RESOURCE_CHANGED_DURING_CHECK",
    )?;
    let python_name = string(&manifest, "python")?;
    let python = root.join(python_name);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        check(
            fs::metadata(&python)?.permissions().mode() & 0o111 != 0,
            "RESOURCE_PYTHON_NOT_EXECUTABLE",
        )?;
    }
    Ok(Verified {
        proof: LaunchProof {
            schema: "local-launcher-proof/1",
            resource_root: root,
            python,
            python_sha256: files[python_name].as_str().unwrap().to_string(),
            manifest_sha256: manifest_hash,
            inventory_files: files.len(),
            code_files: code.len(),
            inventory_verified_before_spawn: true,
            preparation_identity_rechecked: false,
            manifest_trusted: false,
            atomic_fd_exec_proven: false,
            distribution_ready: false,
        },
        stamps,
        root_stamp: root_before,
    })
}

/// Only this type exposes spawn; callers cannot extract a command and skip the
/// last consistency check. Session creation is owned by the App's storage guard.
pub struct PackagedLaunch {
    verified: Verified,
    command: Command,
}
impl PackagedLaunch {
    pub fn prepare(root: &Path, sessions: &Path, legacy: Option<&Path>) -> std::io::Result<Self> {
        check(
            sessions.is_absolute()
                && !sessions
                    .components()
                    .any(|c| matches!(c, std::path::Component::ParentDir)),
            "SESSION_ROOT_ABSOLUTE_REQUIRED",
        )?;
        let verified = verify(root)?;
        let root = &verified.proof.resource_root;
        check(
            !sessions.starts_with(root),
            "SESSION_RESOURCE_ROOT_PROHIBITED",
        )?;
        let mut command = Command::new(&verified.proof.python);
        command
            .args(["-s", "-B", "-m", "scripts.ocr.app_mvp_bridge"])
            .current_dir(root)
            .env_clear();
        for name in ["HOME", "TMPDIR", "LANG", "LC_ALL"] {
            if let Some(value) = std::env::var_os(name) {
                command.env(name, value);
            }
        }
        command
            .env("PATH", "/usr/bin:/bin:/usr/sbin:/sbin")
            .env("PYTHONPATH", root)
            .env("PYTHONNOUSERSITE", "1")
            .env("PYTHONDONTWRITEBYTECODE", "1")
            .env(
                "MUSEION_MVP_RUNTIME_CONFIG",
                root.join("runtime-manifest.json"),
            )
            .env("MUSEION_LOCAL_RESOURCE_ROOT", root)
            .env("MUSEION_LOCAL_SESSION_ROOT", sessions);
        if let Some(path) = legacy {
            check(path.is_absolute(), "SESSION_ROOT_ABSOLUTE_REQUIRED")?;
            command.env("MUSEION_LOCAL_LEGACY_SESSION_ROOT", path);
        }
        Ok(Self { verified, command })
    }
    pub fn proof(&self) -> &LaunchProof {
        &self.verified.proof
    }
    pub fn spawn(mut self) -> std::io::Result<(Child, LaunchProof)> {
        let now = verify(&self.verified.proof.resource_root)?;
        check(
            now.proof.manifest_sha256 == self.verified.proof.manifest_sha256
                && now.stamps == self.verified.stamps
                && now.root_stamp == self.verified.root_stamp,
            "RESOURCE_CHANGED_BEFORE_SPAWN",
        )?;
        self.verified.proof.preparation_identity_rechecked = true;
        let child = self
            .command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        Ok((child, self.verified.proof))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Fixture {
        _temp: tempfile::TempDir,
        root: PathBuf,
        manifest: Value,
    }
    impl Fixture {
        fn new() -> Self {
            let temp = tempfile::tempdir().unwrap();
            let root = temp.path().canonicalize().unwrap().join("resources");
            fs::create_dir(&root).unwrap();
            let mut files = BTreeMap::new();
            let paths: Vec<&str> = SOURCE_FILES
                .iter()
                .copied()
                .chain(INITIALIZERS.iter().copied())
                .chain([
                    "python/bin/python",
                    "bin/helper",
                    "bin/tesseract",
                    "fonts/font.ttf",
                    "tessdata/eng.traineddata",
                    "tessdata/grc.traineddata",
                ])
                .collect();
            for name in &paths {
                let path = root.join(name);
                fs::create_dir_all(path.parent().unwrap()).unwrap();
                fs::write(
                    &path,
                    if INITIALIZERS.contains(name) {
                        b"".as_slice()
                    } else {
                        name.as_bytes()
                    },
                )
                .unwrap();
                files.insert(name.to_string(), sha(&path).unwrap());
            }
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(
                    root.join("python/bin/python"),
                    fs::Permissions::from_mode(0o755),
                )
                .unwrap();
            }
            let manifest = serde_json::json!({"schema":"museion-local-runtime/1","modes":["local"],"python":"python/bin/python","apple_helper":"bin/helper",
                "tesseract":"bin/tesseract","font":"fonts/font.ttf","tessdata":"tessdata","code_files":SOURCE_FILES.iter().map(|k|(k.to_string(),files[*k].clone())).collect::<BTreeMap<_,_>>(),"runtime_files":files});
            let fixture = Self {
                _temp: temp,
                root,
                manifest,
            };
            fixture.write_manifest();
            fixture
        }
        fn write_manifest(&self) {
            fs::write(
                self.root.join("runtime-manifest.json"),
                serde_json::to_vec(&self.manifest).unwrap(),
            )
            .unwrap();
        }
        fn prepare(&self) -> std::io::Result<PackagedLaunch> {
            PackagedLaunch::prepare(
                &self.root,
                &self.root.parent().unwrap().join("unused-sessions"),
                None,
            )
        }
        fn error(&self, expected: &str) {
            match self.prepare() {
                Err(e) => assert!(e.to_string().contains(expected), "{e}"),
                Ok(_) => panic!("unexpected accepted fixture"),
            };
        }
    }
    #[test]
    fn complete_inventory_is_checked_without_creating_sessions_or_starting_python() {
        let f = Fixture::new();
        let launch = f.prepare().unwrap();
        assert_eq!(launch.proof().code_files, 11);
        assert!(!launch.proof().manifest_trusted);
        assert!(!f.root.parent().unwrap().join("unused-sessions").exists());
        assert_eq!(
            launch.command.get_program(),
            f.root.join("python/bin/python")
        );
        assert_eq!(
            launch
                .command
                .get_args()
                .map(|a| a.to_str().unwrap())
                .collect::<Vec<_>>(),
            vec!["-s", "-B", "-m", "scripts.ocr.app_mvp_bridge"]
        );
        let env: BTreeMap<_, _> = launch
            .command
            .get_envs()
            .map(|(k, v)| {
                (
                    k.to_string_lossy().to_string(),
                    v.map(|s| s.to_string_lossy().to_string()),
                )
            })
            .collect();
        assert!(env.keys().all(|k| [
            "HOME",
            "TMPDIR",
            "LANG",
            "LC_ALL",
            "PATH",
            "PYTHONPATH",
            "PYTHONNOUSERSITE",
            "PYTHONDONTWRITEBYTECODE",
            "MUSEION_MVP_RUNTIME_CONFIG",
            "MUSEION_LOCAL_RESOURCE_ROOT",
            "MUSEION_LOCAL_SESSION_ROOT"
        ]
        .contains(&k.as_str())));
        assert!(!env.contains_key("DYLD_INSERT_LIBRARIES"));
        assert!(!env.contains_key("PYTHONHOME"));
    }
    #[test]
    fn replaced_interpreter_is_rejected_before_any_spawn() {
        let f = Fixture::new();
        fs::write(
            f.root.join("python/bin/python"),
            b"#!/bin/sh\ntouch should-never-run\n",
        )
        .unwrap();
        f.error("RESOURCE_HASH_MISMATCH");
        assert!(!f.root.join("should-never-run").exists());
    }
    #[test]
    fn missing_or_extra_code_is_rejected() {
        for extra in [false, true] {
            let mut f = Fixture::new();
            let code = f.manifest["code_files"].as_object_mut().unwrap();
            if extra {
                code.insert(
                    "scripts/ocr/paid_mvp/x.py".into(),
                    Value::String("0".repeat(64)),
                );
            } else {
                code.remove(SOURCE_FILES[0]);
            }
            f.write_manifest();
            f.error("RESOURCE_CODE_WHITELIST");
        }
    }
    #[test]
    fn conflicting_hash_and_missing_model_are_rejected() {
        let mut f = Fixture::new();
        f.manifest["runtime_files"][SOURCE_FILES[0]] = Value::String("0".repeat(64));
        f.write_manifest();
        f.error("RESOURCE_CODE_INVENTORY_CONFLICT");
        let mut f = Fixture::new();
        f.manifest["runtime_files"]
            .as_object_mut()
            .unwrap()
            .remove("tessdata/grc.traineddata");
        f.write_manifest();
        f.error("RESOURCE_TESSDATA_UNHASHED");
    }
    #[test]
    fn undeclared_file_and_bytecode_cannot_be_loaded() {
        for path in ["python/unlisted.py", "python/unlisted.pyc"] {
            let f = Fixture::new();
            fs::write(f.root.join(path), b"extra").unwrap();
            f.error("RESOURCE_INVENTORY_MISSING_OR_EXTRA");
        }
    }
    #[test]
    fn path_escape_alias_and_hooks_are_rejected() {
        for (path, error) in [
            ("../escape", "RESOURCE_PATH_ESCAPE"),
            ("python//evil", "RESOURCE_PATH_ESCAPE"),
            ("python/evil.pth", "RESOURCE_IMPLICIT_RUNTIME_HOOK"),
            ("python/sitecustomize.py", "RESOURCE_IMPLICIT_RUNTIME_HOOK"),
        ] {
            let mut f = Fixture::new();
            f.manifest["runtime_files"][path] = Value::String("0".repeat(64));
            f.write_manifest();
            f.error(error);
        }
    }
    #[test]
    fn declared_nonlocal_script_is_rejected() {
        let mut f = Fixture::new();
        f.manifest["runtime_files"]["scripts/ocr/paid_mvp/selector.py"] =
            Value::String("0".repeat(64));
        f.write_manifest();
        f.error("RESOURCE_NONLOCAL_CODE");
    }
    #[test]
    fn nonempty_bootstrap_initializer_is_rejected_even_with_matching_declared_hash() {
        let mut f = Fixture::new();
        let path = f.root.join(INITIALIZERS[0]);
        fs::write(&path, b"unexpected bootstrap code").unwrap();
        f.manifest["runtime_files"][INITIALIZERS[0]] = Value::String(sha(&path).unwrap());
        f.write_manifest();
        f.error("RESOURCE_BOOTSTRAP_INITIALIZER_NOT_EMPTY");
    }
    #[test]
    fn mode_and_missing_inventory_are_rejected() {
        let mut f = Fixture::new();
        f.manifest["modes"] = serde_json::json!(["local", "paid"]);
        f.write_manifest();
        f.error("RESOURCE_LOCAL_MODE_REQUIRED");
        let mut f = Fixture::new();
        f.manifest.as_object_mut().unwrap().remove("runtime_files");
        f.write_manifest();
        f.error("RESOURCE_IDENTITY_INCOMPLETE");
    }
    #[test]
    fn metadata_changes_after_prepare_are_rejected_even_if_file_hashes_stay_valid() {
        let mut f = Fixture::new();
        let prepared = f.prepare().unwrap();
        f.manifest["config_version"] = Value::String("changed".into());
        f.write_manifest();
        match prepared.spawn() {
            Err(e) => assert!(e.to_string().contains("RESOURCE_CHANGED_BEFORE_SPAWN")),
            Ok(_) => panic!("must not spawn fake interpreter"),
        };
    }
    #[test]
    fn dependency_changes_after_prepare_are_rejected() {
        let f = Fixture::new();
        let prepared = f.prepare().unwrap();
        fs::write(f.root.join("bin/helper"), b"changed").unwrap();
        match prepared.spawn() {
            Err(e) => assert!(e.to_string().contains("RESOURCE_HASH_MISMATCH")),
            Ok(_) => panic!("must not spawn fake interpreter"),
        };
    }
    #[cfg(unix)]
    #[test]
    fn same_bytes_replacement_and_symlink_are_rejected() {
        use std::os::unix::fs::symlink;
        let f = Fixture::new();
        let prepared = f.prepare().unwrap();
        let path = f.root.join("bin/helper");
        let replacement = f.root.parent().unwrap().join("replacement");
        fs::copy(&path, &replacement).unwrap();
        fs::rename(replacement, &path).unwrap();
        match prepared.spawn() {
            Err(e) => assert!(e.to_string().contains("RESOURCE_CHANGED_BEFORE_SPAWN")),
            Ok(_) => panic!("must not spawn fake interpreter"),
        };
        let f = Fixture::new();
        let path = f.root.join("bin/helper");
        fs::remove_file(&path).unwrap();
        symlink(f.root.join("bin/tesseract"), &path).unwrap();
        f.error("RESOURCE_SYMLINK");
    }
    #[test]
    fn session_path_must_be_absolute_and_outside_bundle() {
        let f = Fixture::new();
        assert!(PackagedLaunch::prepare(&f.root, Path::new("relative"), None).is_err());
        assert!(PackagedLaunch::prepare(&f.root, &f.root.join("sessions"), None).is_err());
    }
}
