use predicates::prelude::*;

mod common;

#[test]
fn test_list_empty_or_header() {
    let mut ctx = common::boxlite();
    ctx.cmd
        .arg("list")
        .assert()
        .success()
        .stdout(predicate::str::contains("ID"))
        .stdout(predicate::str::contains("IMAGE"))
        .stdout(predicate::str::contains("STATUS"));
}

#[test]
fn test_list_lifecycle() {
    let mut ctx = common::boxlite();
    let name = "list-lifecycle";

    let _ = ctx
        .cmd
        .args(["create", "--name", name, "alpine:latest"])
        .output();

    ctx.new_cmd()
        .arg("list")
        .assert()
        .success()
        .stdout(predicate::str::contains(name).not());

    ctx.new_cmd()
        .args(["list", "-a"])
        .assert()
        .success()
        .stdout(predicate::str::contains(name))
        .stdout(predicate::str::contains("Configured"));

    ctx.cleanup_box(name);
}

#[test]
fn test_list_alias_ls() {
    let mut ctx = common::boxlite();
    ctx.cmd.arg("ls").assert().success();
}
