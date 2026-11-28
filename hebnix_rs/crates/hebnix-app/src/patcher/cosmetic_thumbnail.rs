use crate::patch_core::upk;
use crate::patcher::upk_package::{ExportEntry, UpkPackage, strip};
use image::{DynamicImage, ImageFormat, RgbaImage};
use std::io::{Cursor, Read, Seek, SeekFrom};
use std::path::Path;

const MAX_TEXTURE_BYTES: usize = 64 * 1024 * 1024;

/// Car-specific engine thumbnail packages often contain only a default asset
/// with no image. Use the associated body's preview, without changing which
/// engine package the swapper installs or restores.
pub(crate) fn body_preview_for_engine(
    name: &str,
    upk: &str,
    bodies: &serde_json::Value,
) -> Option<String> {
    let body_items = bodies.get("bodies")?.as_array()?;
    let engine_key = upk.to_ascii_lowercase();
    let engine_key = engine_key
        .strip_prefix("engineaudio_")?
        .strip_suffix("_sf.upk")?;
    let engine_key = engine_key
        .strip_suffix("_oe")
        .or_else(|| engine_key.strip_suffix("_rev"))
        .unwrap_or(engine_key);
    let by_key: Vec<_> = body_items
        .iter()
        .filter(|body| {
            body.get("upk_path")
                .and_then(serde_json::Value::as_str)
                .is_some_and(|path| {
                    path.to_ascii_lowercase()
                        .strip_prefix("body_")
                        .and_then(|path| path.strip_suffix("_sf.upk"))
                        == Some(engine_key)
                })
        })
        .collect();
    let matches = if by_key.len() == 1 {
        by_key
    } else {
        body_items
            .iter()
            .filter(|body| {
                body.get("name")
                    .and_then(serde_json::Value::as_str)
                    .is_some_and(|body_name| body_name.eq_ignore_ascii_case(name))
            })
            .collect()
    };
    if matches.len() != 1 {
        return None;
    }
    matches[0]
        .get("thumbnail")?
        .as_str()
        .filter(|path| !path.is_empty())
        .map(str::to_owned)
}

fn word(bytes: &[u8], at: &mut usize) -> Result<u32, String> {
    let data = bytes.get(*at..*at + 4).ok_or("Truncated texture record")?;
    *at += 4;
    Ok(u32::from_le_bytes(data.try_into().unwrap()))
}

struct Bulk<'a> {
    flags: u32,
    size: usize,
    disk_size: usize,
    offset: u64,
    inline: &'a [u8],
}

fn bulk<'a>(bytes: &'a [u8], at: &mut usize, offset_width: usize) -> Result<Bulk<'a>, String> {
    let flags = word(bytes, at)?;
    let size = word(bytes, at)? as usize;
    let stored_size = word(bytes, at)?;
    // Stripped cooked mips use -1 for their stored size and have no pixels.
    let disk_size = if flags & 32 != 0 && size == 0 && stored_size == u32::MAX {
        0
    } else {
        stored_size as usize
    };
    if size > MAX_TEXTURE_BYTES || disk_size > MAX_TEXTURE_BYTES {
        return Err(format!(
            "Texture bulk data exceeds size limit: flags={flags:x} size={size} disk={disk_size} at={at}"
        ));
    }
    // Rocket League omits the offset when BULKDATA_NoOffsetFixUp is set.
    let offset = if flags & 0x10000 != 0 && flags & 1 == 0 {
        0
    } else {
        let low = word(bytes, at)? as u64;
        if offset_width == 8 {
            low | ((word(bytes, at)? as u64) << 32)
        } else {
            low
        }
    };
    let inline = if flags & (1 | 32) == 0 {
        let data = bytes
            .get(*at..*at + disk_size)
            .ok_or("Truncated inline mip")?;
        *at += disk_size;
        data
    } else {
        &[]
    };
    Ok(Bulk {
        flags,
        size,
        disk_size,
        offset,
        inline,
    })
}

fn bulk_bytes(record: &Bulk<'_>, directory: &Path, cache: Option<&str>) -> Result<Vec<u8>, String> {
    if record.flags & 32 != 0 || record.size == 0 {
        return Err("Mip has no stored pixels".into());
    }
    let stored = if record.flags & 1 != 0 {
        let cache = cache.ok_or("External mip has no texture cache name")?;
        if cache.contains(['/', '\\', ':']) || cache == ".." {
            return Err("Invalid texture cache name".into());
        }
        let mut file = std::fs::File::open(directory.join(format!("{cache}.tfc")))
            .map_err(|e| format!("{cache}.tfc: {e}"))?;
        file.seek(SeekFrom::Start(record.offset))
            .map_err(|e| e.to_string())?;
        let mut bytes = vec![0; record.disk_size];
        file.read_exact(&mut bytes).map_err(|e| e.to_string())?;
        bytes
    } else {
        record.inline.to_vec()
    };
    let bytes = if record.flags & 2 != 0 {
        upk::decomp_chunk_at(&stored, 0)
            .map_err(|e| format!("Mip decompression: {e:?}"))?
            .0
    } else if record.flags & (16 | 128) != 0 {
        return Err("Unsupported mip compression".into());
    } else {
        stored
    };
    if bytes.len() != record.size {
        return Err("Mip byte count mismatch".into());
    }
    Ok(bytes)
}

fn decode_pixels(
    bytes: &[u8],
    width: usize,
    height: usize,
    format: &str,
) -> Result<RgbaImage, String> {
    if width == 0 || height == 0 || width > 4096 || height > 4096 {
        return Err("Invalid thumbnail dimensions".into());
    }
    if format == "PF_G8" {
        if bytes.len() != width * height {
            return Err("Invalid PF_G8 mip size".into());
        }
        return Ok(RgbaImage::from_fn(width as u32, height as u32, |x, y| {
            let value = bytes[y as usize * width + x as usize];
            image::Rgba([value, value, value, 255])
        }));
    }
    let block_size = match format {
        "PF_DXT1" => 8,
        "PF_DXT3" | "PF_DXT5" | "PF_BC7" => 16,
        "PF_A8R8G8B8" => 0,
        _ => return Err(format!("Unsupported thumbnail pixel format: {format}")),
    };
    let expected = if block_size == 0 {
        width * height * 4
    } else {
        width.div_ceil(4) * height.div_ceil(4) * block_size
    };
    if bytes.len() != expected {
        return Err(format!(
            "Invalid {format} mip size: {} != {expected}",
            bytes.len()
        ));
    }
    let mut rgba = vec![0; width * height * 4];
    if block_size == 0 {
        rgba.copy_from_slice(bytes);
        for pixel in rgba.chunks_exact_mut(4) {
            pixel.swap(0, 2);
        }
    } else {
        for by in 0..height.div_ceil(4) {
            for bx in 0..width.div_ceil(4) {
                let offset = (by * width.div_ceil(4) + bx) * block_size;
                let mut block = [0; 64];
                match format {
                    "PF_DXT1" => bcdec_rs::bc1(&bytes[offset..offset + block_size], &mut block, 16),
                    "PF_DXT3" => bcdec_rs::bc2(&bytes[offset..offset + block_size], &mut block, 16),
                    "PF_BC7" => bcdec_rs::bc7(&bytes[offset..offset + block_size], &mut block, 16),
                    _ => bcdec_rs::bc3(&bytes[offset..offset + block_size], &mut block, 16),
                }
                for y in 0..4.min(height - by * 4) {
                    for x in 0..4.min(width - bx * 4) {
                        let dst = ((by * 4 + y) * width + bx * 4 + x) * 4;
                        let src = (y * 4 + x) * 4;
                        rgba[dst..dst + 4].copy_from_slice(&block[src..src + 4]);
                    }
                }
            }
        }
    }
    RgbaImage::from_raw(width as u32, height as u32, rgba).ok_or("Invalid RGBA image".into())
}

pub(crate) fn texture(
    package: &UpkPackage,
    export: &ExportEntry,
    directory: &Path,
) -> Result<RgbaImage, String> {
    let (props, mut at) = package.serialized_props(export)?;
    let serial = package
        .image
        .get(export.serial_offset..export.serial_offset + export.serial_size)
        .ok_or("Truncated texture")?;
    let name_value = |name: &str| -> Option<&str> {
        let prop = props.iter().find(|p| p.name == name)?;
        let mut pos = prop.value_offset;
        package
            .names
            .get(word(serial, &mut pos).ok()? as usize)
            .map(String::as_str)
    };
    let format = name_value("Format").ok_or("Texture format is missing")?;
    let cache = name_value("TextureFileCacheName");
    // Native Texture2D serialization: source-art bulk, mip count, then each
    // mip's bulk record followed by its actual width and height.
    bulk(serial, &mut at, package.bulk_offset_width())?;
    let count = word(serial, &mut at)?;
    if count == 0 || count > 32 {
        return Err("Invalid thumbnail mip count".into());
    }
    let mut last_error = "No readable thumbnail mip".to_string();
    for _ in 0..count {
        let record = bulk(serial, &mut at, package.bulk_offset_width())?;
        let width = word(serial, &mut at)? as usize;
        let height = word(serial, &mut at)? as usize;
        match bulk_bytes(&record, directory, cache)
            .and_then(|bytes| decode_pixels(&bytes, width, height, format))
        {
            Ok(image) => return Ok(image),
            Err(error) => last_error = error,
        }
    }
    Err(last_error)
}

/// Give uncoloured decal artwork a consistent preview palette. This is a UI
/// treatment, not an in-game paint colour; authored colour thumbnails are kept.
fn colour_decal_preview(image: &mut RgbaImage) {
    // DXT greys can differ slightly between channels because RGB565 gives green
    // an extra bit. Ignore that quantisation, but retain even small colour accents.
    if image.pixels().any(|pixel| {
        let [r, g, b, a] = pixel.0;
        a != 0 && r.max(g).max(b) - r.min(g).min(b) > 12
    }) {
        return;
    }

    // Black stays black, midtones become blue, and highlights become pale cyan.
    // A monotonic ramp preserves the texture's pattern, lighting and contrast.
    const RAMP: [(u16, [u16; 3]); 4] = [
        (0, [0, 0, 0]),
        (85, [15, 55, 100]),
        (170, [35, 155, 220]),
        (255, [210, 245, 255]),
    ];
    for pixel in image.pixels_mut() {
        let [r, g, b, _] = pixel.0;
        let grey = (r as u16 + g as u16 + b as u16) / 3;
        let segment = ((grey / 85) as usize).min(RAMP.len() - 2);
        let (start, low) = RAMP[segment];
        let (end, high) = RAMP[segment + 1];
        for channel in 0..3 {
            pixel[channel] = (low[channel]
                + (high[channel] - low[channel]) * (grey - start) / (end - start))
                as u8;
        }
    }
}

pub fn extract_png(path: &Path, category: &str) -> Result<Vec<u8>, String> {
    let package = UpkPackage::load(path)?;
    let (_, mut pixels) = readable_texture(&package, path, category)?;
    if category == "skins" || category.starts_with("skins|") {
        colour_decal_preview(&mut pixels);
    }
    preview_png(pixels)
}

#[cfg(windows)]
pub(crate) fn extract_arena_preview(path: &Path) -> Result<Vec<u8>, String> {
    let package = UpkPackage::load(path)?;
    let (_, pixels) = readable_texture(&package, path, "arena")?;
    let image = DynamicImage::ImageRgba8(pixels).thumbnail(320, 200).to_rgb8();
    let mut bytes = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut bytes, 80)
        .encode_image(&image)
        .map_err(|error| error.to_string())?;
    Ok(bytes)
}

/// The thumbnail editor displays the stored colours, including greyscale artwork.
pub(crate) fn extract_raw_png(path: &Path, category: &str) -> Result<Vec<u8>, String> {
    let package = UpkPackage::load(path)?;
    let (_, pixels) = readable_texture(&package, path, category)?;
    preview_png(pixels)
}

pub(crate) fn preview_png(pixels: RgbaImage) -> Result<Vec<u8>, String> {
    let image = DynamicImage::ImageRgba8(pixels);
    let image = if image.width() > 512 || image.height() > 512 {
        image.thumbnail(512, 512)
    } else {
        image
    };
    let mut output = Cursor::new(Vec::new());
    image
        .write_to(&mut output, ImageFormat::Png)
        .map_err(|e| e.to_string())?;
    Ok(output.into_inner())
}

/// Resolve exactly the same readable texture for both preview and replacement.
pub(crate) fn readable_texture(
    package: &UpkPackage,
    path: &Path,
    category: &str,
) -> Result<(usize, RgbaImage), String> {
    let requested = category
        .strip_prefix("skins|")
        .and_then(|asset| asset.rsplit('.').next());
    let directory = path.parent().ok_or("Thumbnail has no parent directory")?;
    // Follow ProductThumbnailAsset's reference, rather than choosing whichever
    // Texture2D happens to occur last in a multi-texture package.
    let mut indices = Vec::new();
    let thumbnail_count = package
        .exports
        .iter()
        .filter(|export| {
            package
                .class_of(export)
                .starts_with("ProductThumbnailAsset")
        })
        .count();
    for export in &package.exports {
        if !package
            .class_of(export)
            .starts_with("ProductThumbnailAsset")
        {
            continue;
        }
        if thumbnail_count > 1
            && let Some(leaf) = requested
        {
            let name = package.name_of(export.object_name);
            let name = strip(&name);
            if !name.eq_ignore_ascii_case(&format!("{leaf}_T"))
                && !name.eq_ignore_ascii_case(&format!("{leaf}_TThumbnail"))
            {
                continue;
            }
        }
        if let Ok((props, _)) = package.serialized_props(export) {
            for prop in props
                .iter()
                .filter(|p| p.name == "Thumbnail" && p.tag_type == "ObjectProperty")
            {
                if let Ok(reference) = package.read_int(export.serial_offset + prop.value_offset) {
                    if reference > 0 {
                        indices.push((reference - 1) as usize);
                    }
                }
            }
        }
    }
    if indices.is_empty() && requested.is_none() {
        indices.extend(
            package
                .exports
                .iter()
                .enumerate()
                .filter(|(_, e)| package.class_of(e).starts_with("Texture2D"))
                .map(|(i, _)| i),
        );
    }
    let mut errors = Vec::new();
    for index in indices {
        let Some(export) = package.exports.get(index) else {
            continue;
        };
        if !package.class_of(export).starts_with("Texture2D") {
            continue;
        }
        match texture(&package, export, directory) {
            Ok(image) => return Ok((index, image)),
            Err(error) => errors.push(error),
        }
    }
    Err(format!(
        "No readable thumbnail in {}: {}",
        path.display(),
        errors.join("; ")
    ))
}

/// Bake an opaque image into the existing Texture2D, with a complete inline mip
/// chain. No shared TFC is edited and no absolute bulk-data pointers are emitted.
pub(crate) fn bake_texture(
    package: &UpkPackage,
    export: &ExportEntry,
    pixels: &RgbaImage,
) -> Result<Vec<u8>, String> {
    let bytes = bake_texture_encoded(package, export, pixels, false, false)?;
    make_texture_resident(package, export, bytes, false)
}

/// Bake a mask-bearing diffuse atlas without discarding its alpha channel.
pub(crate) fn bake_texture_alpha(
    package: &UpkPackage,
    export: &ExportEntry,
    pixels: &RgbaImage,
) -> Result<Vec<u8>, String> {
    let bytes = bake_texture_encoded(package, export, pixels, true, false)?;
    make_texture_resident(package, export, bytes, false)
}

pub(crate) fn bake_thumbnail(
    package: &UpkPackage,
    export: &ExportEntry,
    pixels: &RgbaImage,
) -> Result<Vec<u8>, String> {
    let bytes = bake_texture_encoded(package, export, pixels, true, true)?;
    make_texture_resident(package, export, bytes, true)
}

/// Marks rebuilt inline mip data as resident. Retaining the old TFC name lets
/// the game stream unrelated cached pixels over the freshly embedded texture.
fn make_texture_resident(
    package: &UpkPackage,
    export: &ExportEntry,
    mut bytes: Vec<u8>,
    thumbnail: bool,
) -> Result<Vec<u8>, String> {
    let (props, native) = package.serialized_props(export)?;
    let name = |s: &str| {
        package
            .names
            .iter()
            .position(|n| n == s)
            .map(|i| i as u32)
            .ok_or_else(|| format!("Missing texture property name {s}"))
    };
    // These mips are embedded. A retained TFC name permits the streaming path
    // to read the old cache using offsets belonging to the rebuilt package.
    if let Some(cache) = props.iter().find(|p| p.name == "TextureFileCacheName") {
        bytes[cache.value_offset..cache.value_offset + 4]
            .copy_from_slice(&name("None")?.to_le_bytes());
        bytes[cache.value_offset + 4..cache.value_offset + 8].fill(0);
    }
    if let Some(stream) = props.iter().find(|p| p.name == "NeverStream") {
        bytes[stream.value_offset - 1] = 1;
    } else if package.names.iter().any(|n| n == "NeverStream") {
        let mut tag = Vec::new();
        for n in [name("NeverStream")?, 0, name("BoolProperty")?, 0, 0, 0] {
            tag.extend_from_slice(&n.to_le_bytes());
        }
        tag.push(1);
        bytes.splice(native - 8..native - 8, tag);
    } else {
        // Cooked inventory thumbnails omit NeverStream entirely. Their UI LOD
        // group is already non-streaming; keep that group and the inline mips.
        let ui_group = props.iter().find(|p| p.name == "LODGroup").and_then(|p| {
            let mut at = p.value_offset;
            package.names.get(word(&bytes, &mut at).ok()? as usize)
        });
        // Older inventory textures omit both flags. All their mips are inline,
        // FirstResourceMemMip is zero, and no TFC reference remains.
        if !thumbnail && ui_group.map(String::as_str) != Some("TEXTUREGROUP_UI") {
            return Err("Missing texture property name NeverStream".into());
        }
    }
    Ok(bytes)
}

fn encode_dxt5_alpha(pixels: &RgbaImage, width: usize, height: usize) -> Vec<u8> {
    let mut bytes = crate::patcher::patch_core::dxt::image_to_dxt5(pixels, width, height);
    for by in 0..height.div_ceil(4) {
        for bx in 0..width.div_ceil(4) {
            let mut values = [0u8; 16];
            for y in 0..4 {
                for x in 0..4 {
                    values[y * 4 + x] = pixels.get_pixel(
                        (bx * 4 + x).min(width - 1) as u32,
                        (by * 4 + y).min(height - 1) as u32,
                    )[3];
                }
            }
            let hi = *values.iter().max().unwrap();
            let lo = *values.iter().min().unwrap();
            let mut palette = [0u8; 8];
            palette[0] = hi;
            palette[1] = lo;
            if hi > lo {
                for i in 1..7 {
                    palette[i + 1] =
                        (((7 - i) as u16 * hi as u16 + i as u16 * lo as u16) / 7) as u8;
                }
            } else {
                palette.fill(hi);
            }
            let mut indices = 0u64;
            for (i, value) in values.iter().enumerate() {
                let selected = (0..8)
                    .min_by_key(|&j| (palette[j] as i16 - *value as i16).abs())
                    .unwrap();
                indices |= (selected as u64) << (3 * i);
            }
            let at = (by * width.div_ceil(4) + bx) * 16;
            bytes[at] = hi;
            bytes[at + 1] = lo;
            bytes[at + 2..at + 8].copy_from_slice(&indices.to_le_bytes()[..6]);
        }
    }
    bytes
}

/// BC1's transparent palette entry preserves padding in older thumbnails that
/// declare only PF_DXT1. These assets support one-bit alpha.
fn encode_dxt1_alpha(pixels: &RgbaImage, width: usize, height: usize) -> Vec<u8> {
    use crate::patcher::patch_core::dxt;
    let mut output = dxt::image_to_dxt1(pixels, width, height);
    for by in 0..height.div_ceil(4) {
        for bx in 0..width.div_ceil(4) {
            let block: Vec<_> = (0..16)
                .map(|i| {
                    *pixels.get_pixel(
                        (bx * 4 + i % 4).min(width - 1) as u32,
                        (by * 4 + i / 4).min(height - 1) as u32,
                    )
                })
                .collect();
            if block.iter().all(|p| p[3] >= 128) {
                continue;
            }
            let colours: Vec<_> = block
                .iter()
                .filter(|p| p[3] >= 128)
                .map(|p| dxt::rgb_to_rgb565(p[0] as i32, p[1] as i32, p[2] as i32))
                .collect();
            let low = colours.iter().copied().min().unwrap_or(0) as u16;
            let high = colours.iter().copied().max().unwrap_or(0) as u16;
            let (r0, g0, b0) = dxt::rgb565_to_rgb(low);
            let (r1, g1, b1) = dxt::rgb565_to_rgb(high);
            let palette = [
                [r0, g0, b0],
                [r1, g1, b1],
                [(r0 + r1) / 2, (g0 + g1) / 2, (b0 + b1) / 2],
            ];
            let mut indices = 0u32;
            for (i, pixel) in block.iter().enumerate() {
                let index = if pixel[3] < 128 {
                    3
                } else {
                    (0..3)
                        .min_by_key(|&j| {
                            (0..3)
                                .map(|c| (pixel[c] as i32 - palette[j][c]).pow(2))
                                .sum::<i32>()
                        })
                        .unwrap()
                };
                indices |= (index as u32) << (2 * i);
            }
            let at = (by * width.div_ceil(4) + bx) * 8;
            output[at..at + 2].copy_from_slice(&low.to_le_bytes());
            output[at + 2..at + 4].copy_from_slice(&high.to_le_bytes());
            output[at + 4..at + 8].copy_from_slice(&indices.to_le_bytes());
        }
    }
    output
}

fn bake_texture_encoded(
    package: &UpkPackage,
    export: &ExportEntry,
    pixels: &RgbaImage,
    alpha: bool,
    thumbnail: bool,
) -> Result<Vec<u8>, String> {
    let (props, native) = package.serialized_props(export)?;
    let serial = &package.image[export.serial_offset..export.serial_offset + export.serial_size];
    let mut at = native;
    let art = bulk(serial, &mut at, package.bulk_offset_width())?;
    if art.size != 0 {
        return Err("Unexpected source art in cooked ball texture".into());
    }
    let count = word(serial, &mut at)?;
    if count == 0 || count > 32 {
        return Err("Invalid ball mip count".into());
    }
    for _ in 0..count {
        bulk(serial, &mut at, package.bulk_offset_width())?;
        word(serial, &mut at)?;
        word(serial, &mut at)?;
    }
    // Keep the inspected native tail (GUID and empty cached-platform data).
    let tail = &serial[at..];
    let empty_tail: Vec<u8> = [0i32, 0, 0, 33, 0, -1, -1, -1, 0]
        .into_iter()
        .flat_map(i32::to_le_bytes)
        .collect();
    if tail.len() != 52 || tail[16..] != empty_tail {
        return Err(format!(
            "Unsupported cached texture tail ({} bytes): {tail:?}",
            tail.len()
        ));
    }
    let mut output = serial[..native].to_vec();
    let size = pixels
        .width()
        .max(pixels.height())
        .min(1024)
        .next_power_of_two();
    let mips = size.ilog2() + 1;
    let pixel_format = if alpha && !package.names.iter().any(|n| n == "PF_DXT5") {
        // UI thumbnail packages commonly declare only their original BGRA
        // format. Keep it lossless instead of requiring a new name-table entry.
        if package.names.iter().any(|n| n == "PF_A8R8G8B8") {
            "PF_A8R8G8B8"
        } else if thumbnail {
            "PF_DXT1"
        } else {
            return Err("Required alpha-capable texture format name missing".into());
        }
    } else if alpha {
        "PF_DXT5"
    } else {
        "PF_DXT1"
    };
    let format = package
        .names
        .iter()
        .position(|n| n == pixel_format)
        .ok_or("Required alpha-capable texture format name missing")?;
    for prop in &props {
        if alpha && prop.name == "CompressionNoAlpha" {
            output[prop.value_offset - 1] = 0;
        }
        let value = match prop.name.as_str() {
            "SizeX" | "SizeY" | "OriginalSizeX" | "OriginalSizeY" => Some(size),
            "MipTailBaseIdx" => Some(mips - 1),
            "FirstResourceMemMip" => Some(0),
            "Format" => Some(format as u32),
            _ => None,
        };
        if let Some(value) = value {
            output[prop.value_offset..prop.value_offset + 4].copy_from_slice(&value.to_le_bytes());
        }
    }
    let push = |out: &mut Vec<u8>, n: u32| out.extend_from_slice(&n.to_le_bytes());
    for n in [0x10000, 0, 0, mips] {
        push(&mut output, n);
    }
    for level in 0..mips {
        let width = (size >> level).max(1);
        let resized =
            image::imageops::resize(pixels, width, width, image::imageops::FilterType::Triangle);
        let bytes = if pixel_format == "PF_A8R8G8B8" {
            let mut bytes = resized.into_raw();
            for pixel in bytes.chunks_exact_mut(4) {
                pixel.swap(0, 2);
            }
            bytes
        } else if alpha && pixel_format == "PF_DXT1" {
            encode_dxt1_alpha(&resized, width as usize, width as usize)
        } else if alpha {
            encode_dxt5_alpha(&resized, width as usize, width as usize)
        } else {
            crate::patcher::patch_core::dxt::image_to_dxt1(&resized, width as usize, width as usize)
        };
        for n in [0x10000, bytes.len() as u32, bytes.len() as u32] {
            push(&mut output, n);
        }
        output.extend_from_slice(&bytes);
        push(&mut output, width);
        push(&mut output, width);
    }
    output.extend_from_slice(tail);
    Ok(output)
}
