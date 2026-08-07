use anyhow::Result;
use assert_cmd::Command;
use assert_fs::TempDir;
use predicates::prelude::*;

#[test]
fn cancelled_add_exits_with_error() -> Result<()> {
    let directory = TempDir::new()?;
    let database = directory.path().join("coffeedb.db");

    Command::cargo_bin("coffeedb")?
        .args([
            "--database",
            database.to_str().unwrap(),
            "bean",
            "add",
            "filter",
        ])
        .env("EDITOR", "true")
        .assert()
        .failure()
        .code(1)
        .stdout(predicate::str::is_empty())
        .stderr(predicate::str::contains("no data was added"));

    directory.close()?;
    Ok(())
}

#[test]
fn invalid_add_aborted_by_user_exits_with_error() -> Result<()> {
    let directory = TempDir::new()?;
    let database = directory.path().join("coffeedb.db");
    let invalid_record = directory.path().join("invalid.toml");
    std::fs::write(&invalid_record, "this is not valid TOML")?;

    Command::cargo_bin("coffeedb")?
        .args([
            "--database",
            database.to_str().unwrap(),
            "bean",
            "add",
            "omni",
        ])
        .env("EDITOR", format!("cp {}", invalid_record.display()))
        .write_stdin("n\n")
        .assert()
        .failure()
        .code(1)
        .stdout(predicate::str::is_empty())
        .stderr(
            predicate::str::contains("Contents invalid")
                .and(predicate::str::contains("no data was added")),
        );

    directory.close()?;
    Ok(())
}

#[test]
fn duplicate_flavours_return_to_editor() -> Result<()> {
    let directory = TempDir::new()?;
    let database = directory.path().join("coffeedb.db");
    let record = directory.path().join("duplicates.toml");
    std::fs::write(
        &record,
        r#"name = "Test Coffee"
roaster = "Test Roaster"
flavours = ["Apple", "apple"]
"#,
    )?;

    Command::cargo_bin("coffeedb")?
        .args([
            "--database",
            database.to_str().unwrap(),
            "bean",
            "add",
            "filter",
        ])
        .env("EDITOR", format!("cp {}", record.display()))
        .write_stdin("n\n")
        .assert()
        .failure()
        .code(1)
        .stderr(
            predicate::str::contains("duplicate flavour 'apple'")
                .and(predicate::str::contains("Continue editing?"))
                .and(predicate::str::contains("no data was added")),
        );

    directory.close()?;
    Ok(())
}

#[test]
fn cancelled_edit_exits_with_error() -> Result<()> {
    let directory = TempDir::new()?;
    let database = directory.path().join("coffeedb.db");
    let record = directory.path().join("record.toml");
    std::fs::write(
        &record,
        r#"name = "Test Coffee"
roaster = "Test Roaster"
flavours = []
"#,
    )?;

    Command::cargo_bin("coffeedb")?
        .args([
            "--database",
            database.to_str().unwrap(),
            "bean",
            "add",
            "filter",
        ])
        .env("EDITOR", format!("cp {}", record.display()))
        .write_stdin("\n")
        .assert()
        .success();

    Command::cargo_bin("coffeedb")?
        .args([
            "--database",
            database.to_str().unwrap(),
            "bean",
            "edit",
            "F1",
        ])
        .env("EDITOR", "true")
        .assert()
        .failure()
        .code(1)
        .stdout(predicate::str::is_empty())
        .stderr(predicate::str::contains("no data was changed"));

    directory.close()?;
    Ok(())
}

#[test]
fn lists_registered_flavours() -> Result<()> {
    let directory = TempDir::new()?;
    let database = directory.path().join("coffeedb.db");
    let record = directory.path().join("record.toml");
    std::fs::write(
        &record,
        r#"name = "Test Coffee"
roaster = "Test Roaster"
flavours = ["Lemon", "Apple"]
"#,
    )?;

    Command::cargo_bin("coffeedb")?
        .args([
            "--database",
            database.to_str().unwrap(),
            "bean",
            "add",
            "filter",
        ])
        .env("EDITOR", format!("cp {}", record.display()))
        .write_stdin("\n")
        .assert()
        .success();

    Command::cargo_bin("coffeedb")?
        .args([
            "--database",
            database.to_str().unwrap(),
            "bean",
            "list",
            "--flavours",
        ])
        .assert()
        .success()
        .stdout("apple\nlemon\n")
        .stderr(predicate::str::is_empty());

    Command::cargo_bin("coffeedb")?
        .args([
            "--database",
            database.to_str().unwrap(),
            "bean",
            "list",
            "--template",
            "{%flavours}",
        ])
        .assert()
        .success()
        .stdout("lemon, apple\n")
        .stderr(predicate::str::is_empty());

    directory.close()?;
    Ok(())
}

#[test]
fn flavours_conflicts_with_explicit_template() -> Result<()> {
    let directory = TempDir::new()?;
    let database = directory.path().join("coffeedb.db");

    Command::cargo_bin("coffeedb")?
        .args([
            "--database",
            database.to_str().unwrap(),
            "bean",
            "list",
            "--flavours",
            "--template",
            "{%name}",
        ])
        .assert()
        .failure()
        .code(2)
        .stdout(predicate::str::is_empty())
        .stderr(
            predicate::str::contains("--flavours")
                .and(predicate::str::contains("--template"))
                .and(predicate::str::contains("Usage:")),
        );

    directory.close()?;
    Ok(())
}

#[test]
fn invalid_roaster_url_is_reprompted() -> Result<()> {
    let directory = TempDir::new()?;
    let database = directory.path().join("coffeedb.db");
    let record = directory.path().join("record.toml");
    std::fs::write(
        &record,
        r#"name = "Test Coffee"
roaster = "Test Roaster"
flavours = []
"#,
    )?;

    Command::cargo_bin("coffeedb")?
        .args([
            "--database",
            database.to_str().unwrap(),
            "bean",
            "add",
            "filter",
        ])
        .env("EDITOR", format!("cp {}", record.display()))
        .write_stdin("not a url\nhttps://example.com\n")
        .assert()
        .success()
        .stderr(predicate::str::contains("Invalid URL"));

    directory.close()?;
    Ok(())
}

#[test]
fn generates_fish_registration_without_opening_database() -> Result<()> {
    Command::cargo_bin("coffeedb")?
        .env("COMPLETE", "fish")
        .env("COFFEEDB_DATABASE_PATH", "/this/path/must/not/be/opened.db")
        .assert()
        .success()
        .stdout(
            predicate::str::contains("complete --keep-order --exclusive")
                .and(predicate::str::contains("COMPLETE=fish")),
        )
        .stderr(predicate::str::is_empty());

    Ok(())
}

#[test]
fn dynamically_completes_edit_labels_with_fish_help() -> Result<()> {
    let directory = TempDir::new()?;
    let database = directory.path().join("coffeedb.db");
    let record = directory.path().join("record.toml");
    std::fs::write(
        &record,
        r#"name = "Test Coffee"
roaster = "Test Roaster"
flavours = []
"#,
    )?;

    Command::cargo_bin("coffeedb")?
        .args([
            "--database",
            database.to_str().unwrap(),
            "bean",
            "add",
            "filter",
        ])
        .env("EDITOR", format!("cp {}", record.display()))
        .write_stdin("\n")
        .assert()
        .success();

    Command::cargo_bin("coffeedb")?
        .args([
            "--",
            "coffeedb",
            "--database",
            database.to_str().unwrap(),
            "bean",
            "edit",
            "F",
        ])
        .env("COMPLETE", "fish")
        .assert()
        .success()
        .stdout("F1\tTest Coffee (by Test Roaster)\n")
        .stderr(predicate::str::is_empty());

    directory.close()?;
    Ok(())
}

#[test]
fn gets_existing_coffee_as_text() -> Result<()> {
    let directory = TempDir::new()?;
    let database = directory.path().join("coffeedb.db");
    let record = directory.path().join("record.toml");
    std::fs::write(
        &record,
        r#"name = "Test Coffee"
roaster = "Test Roaster"
origin = "Ethiopia"
producer = "Test Producer"
elevation = "1800"
variety = "Bourbon"
process = "Washed"
flavours = ["Peach", "Jasmine"]
"#,
    )?;

    Command::cargo_bin("coffeedb")?
        .args([
            "--database",
            database.to_str().unwrap(),
            "bean",
            "add",
            "filter",
        ])
        .env("EDITOR", format!("cp {}", record.display()))
        .write_stdin("\n")
        .assert()
        .success();

    Command::cargo_bin("coffeedb")?
        .args(["--database", database.to_str().unwrap(), "bean", "get", "F1"])
        .assert()
        .success()
        .stdout("Test Coffee #F1\npeach, jasmine\n\na washed process bourbon\nproduced by Test Producer\nroasted by Test Roaster\n\nEthiopia  1800 masl\n")
        .stderr(predicate::str::is_empty());

    Command::cargo_bin("coffeedb")?
        .args([
            "--database",
            database.to_str().unwrap(),
            "bean",
            "get",
            "F1",
            "-fsvg",
        ])
        .assert()
        .success()
        .stdout(
            predicate::str::starts_with("<svg ")
                .and(predicate::str::contains("Test Coffee"))
                .and(predicate::str::contains("Test Roaster"))
                .and(predicate::str::contains("#F1"))
                .and(predicate::str::ends_with("</svg>\n")),
        )
        .stderr(predicate::str::is_empty());

    Command::cargo_bin("coffeedb")?
        .args([
            "--database",
            database.to_str().unwrap(),
            "bean",
            "get",
            "F1",
            "-ftoml",
        ])
        .assert()
        .success()
        .stdout(
            predicate::str::contains("label = \"F1\"")
                .and(predicate::str::contains("name = \"Test Coffee\""))
                .and(predicate::str::contains("\"peach\"")),
        )
        .stderr(predicate::str::is_empty());

    Command::cargo_bin("coffeedb")?
        .args([
            "--database",
            database.to_str().unwrap(),
            "bean",
            "get",
            "F1",
            "-fjson",
        ])
        .assert()
        .success()
        .stdout(
            predicate::str::starts_with("{\n")
                .and(predicate::str::contains("\"label\": \"F1\""))
                .and(predicate::str::contains("\"name\": \"Test Coffee\""))
                .and(predicate::str::ends_with("}\n")),
        )
        .stderr(predicate::str::is_empty());

    directory.close()?;
    Ok(())
}

#[test]
fn gets_existing_coffee_as_svg_to_file() -> Result<()> {
    let directory = TempDir::new()?;
    let database = directory.path().join("coffeedb.db");
    let record = directory.path().join("record.toml");
    let output = directory.path().join("coffee.svg");
    std::fs::write(
        &record,
        r#"name = "Test Coffee"
roaster = "Test Roaster"
flavours = []
"#,
    )?;

    Command::cargo_bin("coffeedb")?
        .args([
            "--database",
            database.to_str().unwrap(),
            "bean",
            "add",
            "filter",
        ])
        .env("EDITOR", format!("cp {}", record.display()))
        .write_stdin("\n")
        .assert()
        .success();

    Command::cargo_bin("coffeedb")?
        .args([
            "--database",
            database.to_str().unwrap(),
            "bean",
            "get",
            "F1",
            "--format",
            "svg",
            "--out",
            output.to_str().unwrap(),
        ])
        .assert()
        .success()
        .stdout(predicate::str::is_empty())
        .stderr(predicate::str::is_empty());

    let svg = std::fs::read_to_string(&output)?;
    assert!(svg.starts_with("<svg "));
    assert!(svg.contains("Test Coffee"));
    assert!(svg.contains("Test Roaster"));
    assert!(svg.contains("#F1"));
    assert!(svg.ends_with("</svg>\n"));

    directory.close()?;
    Ok(())
}

#[test]
fn printing_to_an_existing_file_exits_with_error() -> Result<()> {
    let directory = TempDir::new()?;
    let database = directory.path().join("coffeedb.db");
    let record = directory.path().join("record.toml");
    let output = directory.path().join("coffee.svg");
    std::fs::write(
        &record,
        r#"name = "Test Coffee"
roaster = "Test Roaster"
flavours = []
"#,
    )?;
    std::fs::write(&output, "existing contents")?;

    Command::cargo_bin("coffeedb")?
        .args([
            "--database",
            database.to_str().unwrap(),
            "bean",
            "add",
            "filter",
        ])
        .env("EDITOR", format!("cp {}", record.display()))
        .write_stdin("\n")
        .assert()
        .success();

    Command::cargo_bin("coffeedb")?
        .args([
            "--database",
            database.to_str().unwrap(),
            "bean",
            "get",
            "F1",
            "-fsvg",
            "--out",
            output.to_str().unwrap(),
        ])
        .assert()
        .failure()
        .code(1)
        .stdout(predicate::str::is_empty())
        .stderr(predicate::str::contains("failed to create output file"));

    assert_eq!(std::fs::read_to_string(&output)?, "existing contents");

    directory.close()?;
    Ok(())
}

#[test]
fn printing_unknown_coffee_exits_with_error() -> Result<()> {
    let directory = TempDir::new()?;
    let database = directory.path().join("coffeedb.db");

    Command::cargo_bin("coffeedb")?
        .args([
            "--database",
            database.to_str().unwrap(),
            "bean",
            "get",
            "F1",
        ])
        .assert()
        .failure()
        .code(1)
        .stdout(predicate::str::is_empty())
        .stderr(predicate::str::contains("coffee 'F1' does not exist"));

    directory.close()?;
    Ok(())
}
