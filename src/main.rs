fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let before = cutback::xml_parser::parse_file(std::path::Path::new(&args[0]))?;
    let after = cutback::xml_parser::parse_file(std::path::Path::new(&args[1]))?;

    let diff = cutback::differ::diff(&before, &after);
    let fps = after.profile.fps();

    println!("summary: {}", cutback::render::summarize(&diff, fps));
    println!();
    for line in cutback::render::render(&diff, fps) {
        println!("  {line}");
    }
    Ok(())
}
