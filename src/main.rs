extern crate ncurses;
extern crate libc;

use ncurses::*;
use image::ImageReader as ImageReader;
use std::time::{Instant, Duration};
use std::thread;

use std::env;
use std::result::*;
use std::process::{Command, Stdio, Child};
use std::fs;
use std::path::Path;

const WHITE_VALUE_THRESHOLD: f32 = 0.5;
const WHITE_SATURATION_THRESHOLD: f32 = 0.09375;

const CHAR_ASPECT: f32 = 0.5;

const BLACKWHITE: bool = false;

struct IMG {
    width: u32,
    height: u32,
    data: Vec<[u8; 3]>,
}

struct PlayerStatus {
    playing: bool,
    width: u32,
    height: u32,
    length: u32,
    position: u32,
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

fn run_command(command: String) -> String {
    let output = if cfg!(target_os = "windows") {
    Command::new("cmd")
        .args(["/C", command.as_str()])
        .output()
        .expect("failed to execute process")
    } else {
        Command::new("sh")
            .arg("-c")
            .arg(command)
            .output()
            .expect("failed to execute process")
    };
    String::from_utf8_lossy(&output.stdout).to_string()
}

fn run_background(command: String) -> Child {
    Command::new("sh")
        .arg("-c")
        .arg(command)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("failed to execute process")
}

fn run_command_visible(command: &str) {
    let _ = Command::new("sh")
        .arg("-c")
        .arg(command)
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .expect("failed to execute process");
}

fn draw_image(image_struct: &IMG, native_size: bool) -> Result<(), Box<dyn std::error::Error>> {
    let mut rows_raw = 0;
    let mut cols_raw = 0;
    getmaxyx(stdscr(), &mut rows_raw, &mut cols_raw);

    let rows = rows_raw as i32;
    let cols = cols_raw as i32;

    let (scaled_height, scaled_width, inc_x, inc_y, x_offset, y_offset) = if native_size {
        let h = image_struct.height as i32;
        let w = image_struct.width as i32;
        (
            h,
            w,
            1.0_f32, 
            1.0_f32,
            (cols - w) / 2, // Centering offset
            -1,
        )
    } else {
        let term_aspect = (rows as f32 / cols as f32) * CHAR_ASPECT;
        let img_aspect = image_struct.height as f32 / image_struct.width as f32;

        let (h, w) = if img_aspect > term_aspect {
            let h_val = rows;
            let w_val = (1.0_f32.max(h_val as f32 / img_aspect / CHAR_ASPECT)) as i32;
            (h_val, w_val)
        } else {
            let w_val = cols;
            let h_val = (1.0_f32.max(w_val as f32 * img_aspect * CHAR_ASPECT)) as i32;
            (h_val, w_val)
        };

        (
            h,
            w,
            (image_struct.width as f32) / (w as f32),
            (image_struct.height as f32) / (h as f32),
            (cols - w) / 2, // Centering offset
            (rows - h) / 2,
        )
    };

    for y in 0..scaled_height {
        let draw_y = y + y_offset;
        if draw_y >= rows || draw_y < 0 { continue; }

        for x in 0..scaled_width {
            let draw_x = x + x_offset;
            if draw_x >= cols || draw_x < 0 { continue; }

            mv(draw_y, draw_x);

            let img_x = if native_size { x as u32 } else { ((x as f32 * inc_x) as u32).min(image_struct.width - 1) };
            let img_y = if native_size { y as u32 } else { ((y as f32 * inc_y) as u32).min(image_struct.height - 1) };
            let index = (img_y * image_struct.width + img_x) as usize;

            if let Some(pixel) = image_struct.data.get(index) {
                let [r, g, b] = *pixel;

                if !BLACKWHITE {
                    let color_idx = rgb_to_16color(r, g, b);
                    attron(COLOR_PAIR((color_idx + 1) as i16));
                    addch(get_brightness_char(r, g, b).chars().next().unwrap_or(' ') as chtype);
                    attroff(COLOR_PAIR((color_idx + 1) as i16));
                } else {
                    addch(get_brightness_char(r, g, b).chars().next().unwrap_or(' ') as chtype);
                }
            }
        }
    }

    Ok(())
}

fn format_time(total_seconds: u32) -> String {
    let f_seconds = total_seconds as f32;

    let hours: f32 = f_seconds / 3600.0;
    let minutes: f32 = (f_seconds % 3600.0) / 60.0;
    let seconds: f32 = f_seconds % 60.0;

    if hours.floor() > 0.0 {
        // Formats as H:MM:SS with leading zeros for M and S
        format!("{}:{:02}:{:02}", hours.floor(), minutes.floor(), seconds.floor())
    } else {
        // Formats as M:SS (standard for shorter videos)
        format!("{:02}:{:02}", minutes.floor(), seconds.floor())
    }
}

fn draw_bar(status: PlayerStatus) -> Result<(), Box<dyn std::error::Error>> {
    let y_pos = status.height as i32 - 1;
    let icon = if status.playing { "|> " } else { "|| " };
    let time = format!(" {}/{} ", format_time(status.position), format_time(status.length));

    let reserved_space = icon.len() as i32 + time.len() as i32 + 2;
    let bar_max_width = (status.width as i32 - reserved_space).max(0);

    let progress = status.position as f32 / status.length as f32;
    let done_size = (bar_max_width as f32 * progress) as usize;
    let todo_size = (bar_max_width as usize).saturating_sub(done_size);

    mv(y_pos, 0);
    clrtoeol(); // Clear the line first so old bars don't ghost
    
    addstr(icon)?;
    addstr(&time)?;
    addstr("[")?;
    addstr(&"#".repeat(done_size))?;
    addstr(&".".repeat(todo_size))?;
    addstr("]")?;

    Ok(())
}

fn play_video(name: &str) -> Result<(), Box<dyn std::error::Error>> {
    init_curses();
    let raw_fps = run_command(format!("ffprobe -v error -select_streams v:0 -show_entries stream=r_frame_rate -of default=noprint_wrappers=1:nokey=1 {}", name));


    // Extract FPS
    let fps = if let Some((num_str, den_str)) = raw_fps.trim().split_once('/') {
        let numerator: f32 = num_str.parse().unwrap_or(30.0);
        let denominator: f32 = den_str.parse().unwrap_or(1.0);
        numerator / denominator
    } else {
        30.0
    };

    let (mut rows, mut cols) = (0, 0);
    getmaxyx(stdscr(), &mut rows, &mut cols);

    let dir = Path::new("imgs");
    if dir.exists() {
        fs::remove_dir_all(dir)?; 
    }
    fs::create_dir_all("imgs/")?;

    let ffmpeg_cmd = format!(
        "ffmpeg -i {} -vf \"scale={}:{}:force_original_aspect_ratio=decrease,scale=iw:ih*0.5,pad={}:{}:(ow-iw)/2:(oh-ih)/2,setsar=1\" -sws_flags neighbor imgs/image%04d.png",
        name, cols, rows * 2 - 3, cols, rows
    );
    run_command_visible(&ffmpeg_cmd);
    
    let frame_duration = Duration::from_nanos((1_000_000_000.0 / fps) as u64);

    let mut audio = run_background(format!("mpv --no-video --quiet --no-terminal {}", name));

    let start_time = Instant::now();
    let mut frame_count = 0;

    let entries: Vec<_> = fs::read_dir("./imgs/")?.filter_map(|r| r.ok()).collect();

    nodelay(stdscr(), true);

    let mut is_paused = false;
    let mut total_paused_duration = Duration::from_secs(0);

    for entry in &entries {
        let key = getch();
        
        if key == 32 { 
            is_paused = !is_paused;

            unsafe { libc::kill(audio.id() as i32, libc::SIGSTOP); } // Freeze mpv

            if is_paused {
                let pause_start = Instant::now();
                while is_paused {
                    let status = PlayerStatus {
                        playing: false,
                        width: cols as u32, // Use terminal size
                        height: rows as u32,
                        position: (frame_count as f32 / fps) as u32,
                        length: (entries.len() as f32 / fps) as u32,
                    };
                    draw_bar(status)?;
                    refresh();

                    thread::sleep(Duration::from_millis(100));

                    if getch() == 32 { is_paused = false; }
                }
            total_paused_duration += pause_start.elapsed();

            unsafe { libc::kill(audio.id() as i32, libc::SIGCONT); } // Resume mpv
            }
        }

        frame_count += 1;
        let target_elapsed = (frame_duration * frame_count) + total_paused_duration;
        let actual_elapsed = start_time.elapsed();

        if actual_elapsed > target_elapsed {
            continue; 
        }

        let path = entry.path();
        if let Some(img) = path.to_str() {
            let img_obj = read_image(img)?;
            // set up status
            let status = PlayerStatus {
                playing: true,
                width: img_obj.width,
                height: img_obj.height,
                position: frame_count / fps as u32,
                length: entries.len() as u32 / fps as u32,
            };

            clear();
            draw_image(&img_obj, true)?;
           
            draw_bar(status)?;

            refresh();
        }

        let final_elapsed = start_time.elapsed();
        if final_elapsed < target_elapsed {
            thread::sleep(target_elapsed - final_elapsed);
        }
    }

    let _ = audio.kill();
    Ok(())
}

fn init_curses() {
    initscr();
    cbreak();
    noecho();
    keypad(stdscr(), true);
    start_color();

    if !has_colors() {
        panic!("Terminal does not support colors");
    }

    for i in 0..16 {
        init_pair((i + 1) as i16, i as i16, 0); 
    }
}


fn main() -> Result<(), Box<dyn std::error::Error>> {
    let argv: Vec<String> = env::args().collect();
    let argc: usize = argv.len();
    
    if argc != 2 {
        println!("Invalid amount of arguments. One required.");
        return Ok(());
    }

    match get_type(&argv[1]) {
        MediaType::Image => {
            init_curses();
            draw_image(&read_image(&argv[1])?, false)?;

            refresh();
            getch();
        }
        MediaType::Video => {
            play_video(&argv[1])?;
        }
        _ => {
            mvaddstr(0, 0, "Format not supported!")?;
        }
    }


    endwin();
    Ok(())
}
