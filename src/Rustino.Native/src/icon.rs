use tao::window::Icon;

fn load_rgba(path: &str) -> Option<(Vec<u8>, u32, u32)> {
    // Detect the format from the content, not the extension
    let reader = image::ImageReader::open(path)
        .ok()?
        .with_guessed_format()
        .ok()?;
    let img = reader.decode().ok()?.into_rgba8();
    let (width, height) = img.dimensions();
    Some((img.into_raw(), width, height))
}

pub fn load_icon(path: &str) -> Option<Icon> {
    let (rgba, width, height) = load_rgba(path)?;
    Icon::from_rgba(rgba, width, height).ok()
}

pub fn load_menu_icon(path: &str) -> Option<muda::Icon> {
    let (rgba, width, height) = load_rgba(path)?;
    muda::Icon::from_rgba(rgba, width, height).ok()
}
