use predicates::prelude::*;

mod common;

#[test]
fn test_create_basic() {
    let mut ctx = common::boxlite();
    ctx.cmd
        .arg("create")
        .arg("alpine:latest")
        .assert()
        .success()
        .stdout(predicate::str::is_match(r"^[0-9A-Z]{26}\n$").unwrap());
}

#[test]
fn test_create_named() {
    let mut ctx = common::boxlite();
    let name = "create-named";
    ctx.cmd
        .arg("create")
        .arg("--name")
        .arg(name)
        .arg("alpine:latest")
        .assert()
        .success()
        .stdout(predicate::str::is_match(r"^[0-9A-Z]{26}\n$").unwrap());

    ctx.new_cmd()
        .arg("create")
        .arg("--name")
        .arg(name)
        .arg("alpine:latest")
        .assert()
        .failure()
        .stderr(predicate::str::contains("already exists"));

    ctx.cleanup_box(name);
}

#[test]
fn test_create_resources() {
    let mut ctx = common::boxlite();
    let name = "create-resources";

    ctx.cmd
        .arg("create")
        .arg("--name")
        .arg(name)
        .arg("--cpus")
        .arg("1")
        .arg("--memory")
        .arg("128")
        .arg("--env")
        .arg("TEST_VAR=1")
        .arg("--workdir")
        .arg("/tmp")
        .arg("alpine:latest")
        .assert()
        .success();

    ctx.cleanup_box(name);
}
