extern crate ncurses;
use ncurses::*;
use image::ImageReader as ImageReader;

use std::env;
use std::result::*;

const WHITE_VALUE_THRESHOLD: f32 = 0.5;
const WHITE_SATURATION_THRESHOLD: f32 = 0.09375;

struct IMG {
    width: u32,
    height: u32,
    data: Vec<[u8; 3]>,
}

#[derive(Debug, PartialEq)]
enum MediaType {
    Image,
    Video,
    None,
}

fn get_type(name: &str) -> MediaType {
    let name_lc = name.to_lowercase();

    let video_formats = [
        ".mp4", ".gif", ".mkv", ".mov", ".webm", ".avi", 
        ".wmv", ".f4v", ".flv", ".m4v", ".3gp"
    ];
    
    let image_formats = [
        ".png", ".jpg", ".jpeg", ".webp", ".tiff", ".ppm", 
        ".pbm", ".pgm", ".bmp", ".ico", ".qoi", ".farbfeld", ".avif"
    ];

    if image_formats.iter().any(|&ext| name_lc.ends_with(ext)) {
        return MediaType::Image;
    }

    if video_formats.iter().any(|&ext| name_lc.ends_with(ext)) {
        return MediaType::Video;
    }

    MediaType::None
}


fn rgb_to_hsv(r: u8, g: u8, b: u8) -> (f32, f32, f32) {
    let r_f = r as f32 / 255.0;
    let g_f = g as f32 / 255.0;
    let b_f = b as f32 / 255.0;

    let max = r_f.max(g_f).max(b_f);
    let min = r_f.min(g_f).min(b_f);
    let delta = max - min;

    let v = max;
    let s = if max == 0.0 { 0.0 } else { delta / max };

    let h = if delta == 0.0 {
        0.0
    } else if max == r_f {
        60.0 * ((g_f - b_f) / delta).rem_euclid(6.0)
    } else if max == g_f {
        60.0 * ((b_f - r_f) / delta) + 120.0
    } else {
        60.0 * ((r_f - g_f) / delta) + 240.0
    };

    (h, s, v)
}

fn hsv_to_rgb(h: f32, s: f32, v: f32) -> (f32, f32, f32) {
    let h_norm = h.rem_euclid(360.0);
    let s_clamp = s.clamp(0.0, 1.0);
    let v_clamp = v.clamp(0.0, 1.0);

    let c = v_clamp * s_clamp;
    let x = c * (1.0 - (((h_norm / 60.0) % 2.0) - 1.0).abs());
    let m = v_clamp - c;

    let (r1, g1, b1) = if h_norm < 60.0 {
        (c, x, 0.0)
    } else if h_norm < 120.0 {
        (x, c, 0.0)
    } else if h_norm < 180.0 {
        (0.0, c, x)
    } else if h_norm < 240.0 {
        (0.0, x, c)
    } else if h_norm < 300.0 {
        (x, 0.0, c)
    } else {
        (c, 0.0, x)
    };

    ((r1 + m) * 255.0, (g1 + m) * 255.0, (b1 + m) * 255.0)
}


fn read_image(path: &str) -> Result<IMG, Box<dyn std::error::Error>> {
    let img = ImageReader::open(path)?.decode()?;
    let mut image_struct: IMG = IMG { 
        width: img.width(), 
        height: img.height(), 
        data: Vec::new(), 
    };

    let rgb = img.to_rgb8();

    for pixel in rgb.pixels() {
        let [r, g, b] = pixel.0;
        image_struct.data.push([r, g, b]);
    }

    Ok(image_struct)
}


fn get_brightness(r: u8, g: u8, b: u8) -> f32 {
    let r_f = r as f32;
    let g_f = g as f32;
    let b_f = b as f32;
    0.2126 * r_f + 0.7152 * g_f + 0.0722 * b_f
}

fn get_brightness_char(r: u8, g: u8, b: u8) -> String {
    let chars = " .,\";*%#$";

    let chars_vec: Vec<char> = chars.chars().collect();
    let max_index = chars_vec.len().saturating_sub(1);

    let index = ((get_brightness(r, g, b) / 255.0) * (max_index as f32)) as usize;
    chars_vec[index].to_string()
}

fn rgb_to_16color(r: u8, g: u8, b: u8) -> usize {
    let std_16_colors: [(u8, u8, u8); 16] = [
        (0, 0, 0), (128, 0, 0), (0, 128, 0), (128, 128, 0),
        (0, 0, 128), (128, 0, 128), (0, 128, 128), (192, 192, 192),
        (128, 128, 128), (255, 0, 0), (0, 255, 0), (255, 255, 0),
        (0, 0, 255), (255, 0, 255), (0, 255, 255), (255, 255, 255)
    ];

    let (h, s, v): (f32, f32, f32) = rgb_to_hsv(r, g, b);

    if v > WHITE_VALUE_THRESHOLD && s < WHITE_SATURATION_THRESHOLD {
        return 15; // White
    }

    // We re-calculate RGB from this "vibrant" HSV
    let (rn, gn, bn): (f32, f32, f32) = hsv_to_rgb(h, 1.0, v);

    let mut best_idx = 0;
    let mut best_dist = f32::INFINITY;

    for (idx, &(cr, cg, cb)) in std_16_colors.iter().enumerate() {
        // Casting to f32 here prevents the subtraction overflow panic
        let dist = (rn - cr as f32).powi(2) * 0.3
                 + (gn - cg as f32).powi(2) * 0.59
                 + (bn - cb as f32).powi(2) * 0.11;
        
        if dist < best_dist {
            best_dist = dist;
            best_idx = idx;
        }
    }

    best_idx
}


fn draw_image(image_struct: IMG) -> Result<(), Box<dyn std::error::Error>> {
    let mut rows = 0;
    let mut cols = 0;

    getmaxyx(stdscr(), &mut rows, &mut cols);
    
    let inc_x: f32 = (image_struct.width as f32) / (cols as f32);
    let inc_y: f32 = (image_struct.height as f32) / (rows as f32);

    for y in 0..rows {
        for x in 0..cols {
            mv(y, x);
            let img_x = (x as f32 * inc_x) as u32;
            let img_y = (y as f32 * inc_y) as u32;
            let index = (img_y * image_struct.width + img_x) as usize;

            if let Some(pixel) = image_struct.data.get(index) {
                let [r, g, b] = *pixel;
                let color_idx = rgb_to_16color(r, g, b);
                attron(COLOR_PAIR((color_idx + 1) as i16));
                addstr(get_brightness_char(r, g, b).as_str())?;
                attroff(COLOR_PAIR((color_idx + 1) as i16));
            }
        }
    }

    Ok(())
}


fn main() -> Result<(), Box<dyn std::error::Error>> {
    let argv: Vec<String> = env::args().collect();
    let argc: usize = argv.len();
    
    if argc != 2 {
        println!("Invalid amount of arguments. One required.");
        return Ok(());
    }

    initscr();
    cbreak();
    noecho();
    keypad(stdscr(), true);
    start_color();

    if !has_colors() {
        endwin();
        panic!("Terminal does not support colors");
    }

    for i in 0..16 {
        init_pair((i + 1) as i16, i as i16, 0); 
    }

    match get_type(&argv[1]) {
        MediaType::Image => {
            draw_image(read_image(&argv[1])?)?;
        }
        MediaType::Video => {
            mvaddstr(0, 0, "Video not supported yet!")?;
        }
        _ => {
            mvaddstr(0, 0, "Format not supported!")?;
        }
    }

    refresh();
    getch();

    endwin();
    Ok(())
}
