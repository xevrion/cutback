fn main() -> anyhow::Result<()> {
    let path = std::env::args().nth(1).unwrap_or_default();
    let project = cutback::xml_parser::parse_file(std::path::Path::new(&path))?;
    println!("{project:#?}");
    Ok(())
}
