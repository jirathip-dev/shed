use std::os::unix::fs::PermissionsExt as _;
use std::path::Path;

use serde_json::Value;

use crate::common::{Server, temp_dir};
use crate::{shed, text};

const SAME: &str = "cred-same-5f1e";
const OTHER: &str = "cred-other-9c3a";
const LOGIN: &str = "keychain and .credentials.json disagree: run shed account login fake";

fn record(home: &Path) -> String {
    let security = home.join("security");
    format!(
        "id = \"fake\"\nagent = \"fake\"\nconfig_dir_var = \"FAKE_HOME\"\nstatus = \"echo in\"\n\
         [credentials]\nfile = \".credentials.json\"\ndefault_dir = \".fake\"\n\
         service = \"Fake-credentials\"\nfolder_hash = 8\nkeychain = [\"{}\", \"{{service}}\"]\nmissing = 44\n",
        security.display()
    )
}

fn security(home: &Path) {
    let script = format!(
        "#!/bin/sh\necho \"$1\" >> '{h}/asked'\ntest -f '{h}/locked' && exit 36\n\
         test -f '{h}/items/'\"$1\" || exit 44\ncat '{h}/items/'\"$1\"\n",
        h = home.display()
    );
    let file = home.join("security");
    std::fs::write(&file, script).expect("fake security");
    std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    std::fs::create_dir_all(home.join("items")).expect("items");
}

fn status(home: &Path, name: &str, seen: &mut String) -> Value {
    let d = home.to_str().expect("utf-8");
    let out = shed(
        home,
        &["--dir", d, "--json", "account", "status", "fake", name],
    );
    assert!(out.status.success(), "{out:?}");
    let out = text(&out);
    seen.push_str(&out);
    serde_json::from_str(&out).expect("JSON")
}

fn files(dir: &Path, skip: &[&Path], all: &mut String) {
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let path = entry.path();
        if skip.contains(&path.as_path()) {
            continue;
        }
        if path.is_dir() && !path.is_symlink() {
            files(&path, skip, all);
        } else if let Ok(bytes) = std::fs::read(&path) {
            all.push_str(&String::from_utf8_lossy(&bytes));
        }
    }
}

#[test]
fn drift_between_the_keychain_and_the_credentials_file_marks_the_account() {
    let home = temp_dir("drift");
    let h = home.to_str().expect("utf-8");
    security(&home);
    std::fs::create_dir_all(home.join("providers")).expect("providers");
    std::fs::write(home.join("providers/fake.toml"), record(&home)).expect("record");
    let _server = Server::with_env(&home, "3600", &[("HOME", h)]);
    let (cred, item) = (
        home.join(".fake/.credentials.json"),
        home.join("items/Fake-credentials"),
    );
    std::fs::create_dir_all(home.join(".fake")).expect("default folder");
    let mut seen = String::new();

    std::fs::write(&cred, SAME).expect("file");
    std::fs::write(&item, format!("{SAME}\n")).expect("item");
    let same = status(&home, "default", &mut seen);
    assert_eq!(
        (&same["state"], &same["drift"]),
        (&"ready".into(), &"clear".into()),
        "{same}"
    );

    std::fs::write(&item, OTHER).expect("item");
    let drifted = status(&home, "default", &mut seen);
    assert_eq!(drifted["state"], "drifted", "{drifted}");
    assert_eq!(drifted["line"], format!("{LOGIN} default"));
    let d = home.to_str().expect("utf-8");
    let ls = text(&shed(&home, &["--dir", d, "account", "ls"]));
    assert!(ls.contains("fake/default  drifted  "), "{ls}");
    let doctor = text(&shed(&home, &["--dir", d, "doctor"]));
    let row = format!("account  fake/default drifted: {LOGIN} default\n");
    assert!(doctor.contains(&row), "{doctor}");
    seen.push_str(&ls);
    seen.push_str(&doctor);
    seen.push_str(&text(&shed(
        &home,
        &["--dir", d, "--json", "account", "ls"],
    )));

    std::fs::remove_file(&cred).expect("rm file");
    let missing = status(&home, "default", &mut seen);
    assert_eq!(
        (&missing["state"], &missing["drift"]),
        (&"ready".into(), &"clear".into())
    );

    std::fs::write(&cred, SAME).expect("file");
    std::fs::write(home.join("locked"), "").expect("lock");
    let unknown = status(&home, "default", &mut seen);
    assert_eq!(unknown["drift"], "unknown", "{unknown}");
    assert_eq!(unknown["line"], "keychain not readable: drift unknown");
    std::fs::remove_file(home.join("locked")).expect("unlock");

    let out = shed(&home, &["--dir", d, "account", "add", "fake", "work"]);
    assert!(out.status.success(), "{out:?}");
    let folder = home.join("accounts/fake/work");
    std::fs::write(folder.join(".credentials.json"), SAME).expect("file");
    status(&home, "work", &mut seen);
    let asked = std::fs::read_to_string(home.join("asked")).expect("asked");
    let service = asked.lines().last().expect("a service").to_owned();
    let hash = service.strip_prefix("Fake-credentials-").expect("suffixed");
    assert!(
        hash.len() == 8 && hash.bytes().all(|b| b.is_ascii_hexdigit()),
        "{service}"
    );
    std::fs::write(home.join("items").join(&service), OTHER).expect("item");
    let work = status(&home, "work", &mut seen);
    assert_eq!(work["line"], format!("{LOGIN} work"), "{work}");

    let inputs = [
        cred.as_path(),
        &home.join("items"),
        &folder.join(".credentials.json"),
    ];
    files(&home, &inputs, &mut seen);
    for secret in [SAME, OTHER] {
        assert!(!seen.contains(secret), "{secret} leaked");
    }
}
