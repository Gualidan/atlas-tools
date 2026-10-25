use anyhow::Result;
use sisyphus::extract::source_root::find_source_root;
use std::{fs, os::unix::fs::symlink};

#[test]
fn a_single_directory_is_the_source_root_without_recursing_further() -> Result<()> {
    let tmp = tempfile::tempdir()?;
    let extracted = tmp.path().to_path_buf();
    let package = extracted.join("example-1.0.0");
    fs::create_dir_all(package.join("src"))?;
    assert_eq!(find_source_root(&extracted)?, package);
    Ok(())
}

#[test]
fn an_empty_archive_keeps_the_extraction_directory() -> Result<()> {
    let tmp = tempfile::tempdir()?;
    let extracted = tmp.path().to_path_buf();
    assert_eq!(find_source_root(&extracted)?, extracted);
    Ok(())
}

#[test]
fn a_single_file_keeps_the_extraction_directory() -> Result<()> {
    let tmp = tempfile::tempdir()?;
    let extracted = tmp.path().to_path_buf();
    fs::write(extracted.join("source.c"), b"int main(void) {}")?;
    assert_eq!(find_source_root(&extracted)?, extracted);
    Ok(())
}

#[test]
fn multiple_directories_keep_the_extraction_directory() -> Result<()> {
    let tmp = tempfile::tempdir()?;
    let extracted = tmp.path().to_path_buf();
    fs::create_dir(extracted.join("src"))?;
    fs::create_dir(extracted.join("docs"))?;
    assert_eq!(find_source_root(&extracted)?, extracted);
    Ok(())
}

#[test]
fn a_directory_with_a_sibling_file_keeps_the_extraction_directory() -> Result<()> {
    let tmp = tempfile::tempdir()?;
    let extracted = tmp.path().to_path_buf();
    fs::create_dir(extracted.join("src"))?;
    fs::write(extracted.join("README"), b"example")?;
    assert_eq!(find_source_root(&extracted)?, extracted);
    Ok(())
}

#[test]
fn a_single_symlink_to_a_directory_is_not_a_source_root() -> Result<()> {
    let tmp = tempfile::tempdir()?;
    let target = tempfile::tempdir()?;
    let extracted = tmp.path().to_path_buf();
    symlink(target.path(), extracted.join("link"))?;
    assert_eq!(find_source_root(&extracted)?, extracted);
    Ok(())
}

#[test]
fn a_missing_or_non_directory_extraction_path_reports_an_error() -> Result<()> {
    let tmp = tempfile::tempdir()?;
    let file = tmp.path().join("file");
    fs::write(&file, b"contents")?;
    for path in [tmp.path().join("missing"), file] {
        let err = find_source_root(&path).expect_err("path must name an existing directory");
        assert!(
            format!("{err:#}").contains("Failed to read extracted directory"),
            "{err:#}"
        );
    }
    Ok(())
}
