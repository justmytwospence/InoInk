mod common;
use common::*;
use inoink::accounts::*;
use inoink::api::auth::AppConfig;
use inoink::api::TokenStore;
use inoink::credentials::{Credentials, FileCredentials};
use inoink::share::{load_config, KarakeepConfig};

fn creds(user: &str) -> Credentials {
    let cfg = AppConfig::new("id", "secret", RELAY_REDIRECT_URI);
    let mut c = Credentials::new(&cfg, &tokens(100));
    c.user_id = user.into();
    c.user_name = "Ann".into();
    c
}

fn karakeep() -> KarakeepConfig {
    KarakeepConfig { url: "https://keep.example".into(), api_key: "k".into() }
}

#[test]
fn transfer_round_trip_and_merge() {
    let dir = tempfile::tempdir().unwrap();
    write_transfer(dir.path(), &Transfer { karakeep: Some(karakeep()), ..Default::default() }).unwrap();
    write_transfer(dir.path(), &Transfer { inoreader: Some(creds("1001")), ..Default::default() }).unwrap();
    let t = read_transfer(dir.path()).unwrap().unwrap();
    assert!(t.inoreader.is_some() && t.karakeep.is_some());
    assert_eq!(import_provisioned(dir.path()).unwrap(), vec![Service::Inoreader, Service::Karakeep]);
    let c = FileCredentials::new(dir.path()).load_credentials().unwrap().unwrap();
    assert_eq!(c.user_name, "Ann");
    assert_eq!(load_config(dir.path()).unwrap().karakeep, Some(karakeep()));
    assert!(!dir.path().join(TRANSFER).exists());
    assert!(import_provisioned(dir.path()).unwrap().is_empty());
}

#[test]
fn import_ignores_unknown_services() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("provision")).unwrap();
    std::fs::write(dir.path().join(TRANSFER), r#"{"readwise":{"token":"x"},"karakeep":{"url":"https://k","api_key":"k"}}"#).unwrap();
    assert_eq!(import_provisioned(dir.path()).unwrap(), vec![Service::Karakeep]);
}

#[test]
fn damaged_transfer_is_deleted_with_error() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("provision")).unwrap();
    std::fs::write(dir.path().join(TRANSFER), "nope").unwrap();
    assert!(import_provisioned(dir.path()).is_err());
    assert!(!dir.path().join(TRANSFER).exists());
}

#[test]
fn legacy_age_files_are_deleted() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("provision");
    std::fs::create_dir_all(&p).unwrap();
    for f in ["credentials.age", "share.age", "device.pub"] {
        std::fs::write(p.join(f), "x").unwrap();
    }
    std::fs::write(dir.path().join("device.key"), "x").unwrap();
    let e = import_provisioned(dir.path()).unwrap_err().to_string();
    assert!(e.contains("older inoink-cli"), "{e}");
    for f in ["credentials.age", "share.age", "device.pub"] {
        assert!(!p.join(f).exists());
    }
    assert!(!dir.path().join("device.key").exists());
}

#[test]
fn shared_fixture_parses() {
    let t: Transfer = serde_json::from_str(include_str!("../../../tests/fixtures/accounts.json")).unwrap();
    assert_eq!(t.inoreader.unwrap().user_name, "Ann");
    assert_eq!(t.karakeep.unwrap().api_key, "fixture-key");
}

#[test]
fn sign_in_inoreader_fills_name() {
    let mock = Mock::new();
    mock.push(Ok(resp(200, r#"{"access_token":"a","refresh_token":"r","expires_in":3600,"scope":"read write"}"#)));
    mock.push(Ok(resp(200, r#"{"userId":"1001","userName":"Ann","userEmail":"a@x"}"#)));
    let cfg = AppConfig::new("id", "secret", RELAY_REDIRECT_URI);
    let c = sign_in_inoreader(&cfg, mock, FixedClock::new(10), "code").unwrap();
    assert_eq!(c.user_id, "1001");
    assert_eq!(c.user_name, "Ann");
    assert_eq!(c.access_token, "a");
}

#[test]
fn sign_out_each_service() {
    let dir = tempfile::tempdir().unwrap();
    FileCredentials::new(dir.path()).save_credentials(&creds("1")).unwrap();
    inoink::share::set_karakeep(dir.path(), Some(karakeep())).unwrap();
    assert_eq!(describe(dir.path(), Service::Inoreader).unwrap(), "Signed in as Ann");
    assert_eq!(describe(dir.path(), Service::Karakeep).unwrap(), "Sends to keep.example");
    sign_out(dir.path(), Service::Inoreader).unwrap();
    sign_out(dir.path(), Service::Karakeep).unwrap();
    assert!(describe(dir.path(), Service::Inoreader).is_none());
    assert!(describe(dir.path(), Service::Karakeep).is_none());
}

#[test]
fn file_credentials_guard_refuses_other_account() {
    let dir = tempfile::tempdir().unwrap();
    FileCredentials::new(dir.path()).save_credentials(&creds("a")).unwrap();
    let before = std::fs::read(dir.path().join("credentials.json")).unwrap();
    let t = tokens(999);
    assert!(FileCredentials::for_account(dir.path(), "b").save(&t).is_err());
    assert_eq!(std::fs::read(dir.path().join("credentials.json")).unwrap(), before);
    FileCredentials::for_account(dir.path(), "a").save(&t).unwrap();
}

#[test]
fn relay_state_format() {
    assert_eq!(relay_state("ab", "192.168.1.5", 8080), "ab_192.168.1.5_8080");
    assert_eq!(Service::parse(" Karakeep "), Some(Service::Karakeep));
    assert_eq!(Service::parse("x"), None);
}
