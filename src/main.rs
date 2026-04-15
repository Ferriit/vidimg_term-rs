extern crate ncurses;
extern crate libc;

use ncurses::*;
use ncurses::setlocale;
use image::ImageReader as ImageReader;
use std::time::{Instant, Duration};
use std::thread;

use std::env;
use std::result::*;
use std::process::{Command, Stdio, Child};
use std::fs;
use std::path::Path;

use std::io::Write;
use std::os::unix::net::UnixStream;
use glob::glob;


const WHITE_VALUE_THRESHOLD: f32 = 0.25;
const WHITE_SATURATION_THRESHOLD: f32 = 0.1;//0.09375;

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
    volume: u32,
}

#[derive(Debug, PartialEq)]
enum MediaType {
    Image,
    Video,
    None,
    Folder,
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

    let path = Path::new(name);

    if path.is_dir() {
        return MediaType::Folder;
    }

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

fn get_brightness_char(r: u8, g: u8, b: u8, unicode: bool) -> String {
    let chars = if !unicode {
        " .,\";*%#$"
    }
    else {
        "  .:-=+*#%@"
    };

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

    let (rn, gn, bn): (f32, f32, f32) = hsv_to_rgb(h, 1.0, v);

    let mut best_idx = 0;
    let mut best_dist = f32::INFINITY;

    for (idx, &(cr, cg, cb)) in std_16_colors.iter().enumerate() {
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

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    if !stderr.is_empty() {
        eprintln!("YT-DLP ERROR:\n{}", stderr);
    }

    stdout.to_string()
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

#[allow(dead_code)]
fn run_command_visible(command: &str) {
    let _ = Command::new("sh")
        .arg("-c")
        .arg(command)
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .expect("failed to execute process");
}

fn draw_image(image_struct: &IMG, native_size: bool, unicode: bool) -> Result<(), Box<dyn std::error::Error>> {
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
                let color_idx = rgb_to_16color(r, g, b);

                if !BLACKWHITE && color_idx != 7 && color_idx != 8 && color_idx != 15 {
                    attron(COLOR_PAIR((color_idx + 1) as i16));
                    addch(get_brightness_char(r, g, b, unicode).chars().next().unwrap_or(' ') as chtype);
                    attroff(COLOR_PAIR((color_idx + 1) as i16));
                } else {
                    addch(get_brightness_char(r, g, b, unicode).chars().next().unwrap_or(' ') as chtype);
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

    mv(y_pos - 1, 0);
    addstr(format!("Vol: {}%", status.volume).as_str())?;

    Ok(())
}


fn mpv_command(socket: &str, cmd: &str) {
    if let Ok(mut stream) = UnixStream::connect(socket) {
        let _ = stream.write_all(format!("{}\n", cmd).as_bytes());
    }
}

fn play_video(name: &str, unicode: bool) -> Result<(), Box<dyn std::error::Error>> {
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
        "ffmpeg -i \"{}\" -vf \"scale={}:{}:force_original_aspect_ratio=decrease,scale=iw:ih*0.5,pad={}:{}:(ow-iw)/2:(oh-ih)/2,setsar=1\" -sws_flags neighbor imgs/image%50d.png -threads 0",
        name, cols, rows * 2 - 3, cols, rows
    );
    clear();
    mvaddstr(0, 0, "Processing video")?;
    refresh();
    run_command(ffmpeg_cmd);

    let frame_duration = Duration::from_nanos((1_000_000_000.0 / fps) as u64);

    let socket_path = "/tmp/mpv_socket";

    let mut audio = run_background(format!(
        "mpv --no-video --quiet --no-terminal --input-ipc-server={} \"{}\"",
        socket_path, name
    ));

    let mut volume = 100;

    let mut start_time = Instant::now();
    let mut frame_count = 0;

    let mut entries: Vec<_> = fs::read_dir("./imgs/")?.filter_map(|r| r.ok()).collect();
    entries.sort_by_key(|entry| entry.path());

    nodelay(stdscr(), true);

    let mut is_paused = false;
    let total_paused_duration = Duration::from_secs(0);

    while frame_count < entries.len() {
        let entry = &entries[frame_count];
        let key = getch();
        
        if key == 32 {
            is_paused = !is_paused;

            if is_paused {
                let pause_start = Instant::now();

                mpv_command(socket_path, r#"{"command": ["set_property", "pause", true]}"#);

                while is_paused {
                    let status = PlayerStatus {
                        playing: false,
                        width: cols as u32,
                        height: rows as u32,
                        position: (frame_count as f32 / fps) as u32,
                        length: (entries.len() as f32 / fps) as u32,
                        volume: volume
                    };

                    draw_bar(status)?;
                    refresh();

                    thread::sleep(Duration::from_millis(100));

                    if getch() == 32 {
                        is_paused = false;
                    }
                }

                start_time += pause_start.elapsed();

                mpv_command(socket_path, r#"{"command": ["cycle", "pause"]}"#);
            }
        }
        // TIME CHANGES
        else if key == KEY_RIGHT {
            frame_count += fps as usize * 5;

            let seconds = frame_count as f32 / fps;
            let cmd = format!(
                "{{\"command\": [\"set_property\", \"time-pos\", {}]}}\n",
                seconds
            );
            mpv_command(socket_path, &cmd);

            start_time = Instant::now() - (frame_duration * frame_count as u32);
        }
        else if key == KEY_LEFT {
            frame_count = frame_count.saturating_sub(fps as usize * 5);

            let seconds = frame_count as f32 / fps;
            let cmd = format!(
                "{{\"command\": [\"set_property\", \"time-pos\", {}]}}\n",
                seconds
            );
            mpv_command(socket_path, &cmd);

            start_time = Instant::now() - (frame_duration * frame_count as u32);
        }
        // VOLUME CHANGES
        else if key == KEY_UP && volume < 100 {
            mpv_command(socket_path, r#"{"command": ["add", "volume", 5]}"#);
            volume += 5;
        }
        else if key == KEY_DOWN && volume >= 5 {
            mpv_command(socket_path, r#"{"command": ["add", "volume", -5]}"#);
            volume -= 5;
        }

        else if key == 'q' as i32 {
            // QUIT 
            let _ = audio.kill();

            // cleanup
            let dir = Path::new("imgs");
            if dir.exists() {
                fs::remove_dir_all(dir)?; 
            }

            // exit
            return Ok(());
        }


        frame_count += 1;
        let target_elapsed = (frame_duration * frame_count as u32) + total_paused_duration;
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
                position: (frame_count / fps as usize) as u32,
                length: entries.len() as u32 / fps as u32,
                volume: volume
            };

            clear();
            draw_image(&img_obj, true, unicode)?;
           
            draw_bar(status)?;

            refresh();
        }

        let final_elapsed = start_time.elapsed();
        if final_elapsed < target_elapsed {
            thread::sleep(target_elapsed - final_elapsed);
        }
    }

    let _ = audio.kill();

    // cleanup
    let dir = Path::new("imgs");
    if dir.exists() {
        fs::remove_dir_all(dir)?; 
    }

    Ok(())
}

fn image_roll(name: &str, unicode: bool) -> Result<(), Box<dyn std::error::Error>> {
    init_curses();

    let path = Path::new(name);

    if path.is_file() { // This function is only called for folders and images
        draw_image(&read_image(name)?, false, unicode)?;
        getch();
    } else if path.is_dir() {
        let mut entries: Vec<_> = fs::read_dir(name)?
            .filter_map(|r| r.ok())
            .collect();
        entries.sort_by_key(|entry| entry.path());

        let mut idx = 0;
        let len = entries.len();

        let mut prev_video: String = String::new();

        loop {
            let path = entries[idx].path();
            if let Some(path_str) = path.to_str() {
                clear();
                match get_type(path_str) {
                    MediaType::Image => {
                        // Show loading bar
                        mvaddstr(0, 0, "+-------+\n|Loading|\n+-------+")?;
                        refresh();

                        // Show image
                        clear();
                        draw_image(&read_image(path_str)?, false, unicode)?;
                        refresh();
                    }
                    MediaType::Video => {
                        if prev_video != path_str {
                            clear();
                            mvaddstr(0, 0, format!("Show video '{}'? (y/n){}", path_str, " ".repeat(50)).as_str())?;
                            refresh();

                            if getch() == 'y' as i32 {
                                clear();
                                play_video(path_str, unicode)?;

                                // Flush leftover key presses
                                while getch() != ERR {} // clear any buffered input
                                nodelay(stdscr(), false);
                            }

                            prev_video = path_str.to_string();
                        }
                    }
                    _ => { idx = (idx + 1) % len; continue; } // skip unknown files
                }
                
                mvaddstr(0, 0, path_str)?;
                refresh();
            }


            let key = getch();
            if key == 27 || key == 'q' as i32 { break; }
            else if key == KEY_RIGHT { idx = (idx + 1) % len; }
            else if key == KEY_LEFT { idx = (idx + len - 1) % len; }

            // thread::sleep(Duration::from_millis(50));
        }
    } else {
        mvaddstr(0, 0, "Unable to read path!")?;
        getch();
    }

    endwin();
    Ok(())
}


fn strip_unicode(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_ascii())
        .collect()
}


fn load_videos(query: &str, page: usize, page_size: usize) -> Vec<(String, String)> {
    let start = page * page_size + 1;
    let end = start + page_size - 1;

    let cmd = format!(
        "yt-dlp \"ytsearch{}:{}\" --playlist-items {}:{} --print \"%(title)s|%(webpage_url)s\"",
        end, query, start, end
    );

    let output = run_command(cmd);

    output
        .lines()
        .filter_map(|line| {
            let (title, url) = line.rsplit_once('|')?;

            Some((
                strip_unicode(title),
                url.to_string(),
            ))
        })
        .collect()
}


fn find_downloaded_file() -> Option<String> {
    let pattern = "vidimg_yt_dlp/yt.*";

    if let Ok(mut entries) = glob(pattern) {
        if let Some(Ok(path)) = entries.next() {
            return Some(path.display().to_string());
        }
    }
    
    None
}


fn youtube_ui() -> Result<(), Box<dyn std::error::Error>> {
    let mut searching = false;
    let mut query: String = String::new();

    let mut results: Vec<(String, String)> = Vec::new();

    let mut width = 0;
    let mut height = 0;

    let mut update = false;

    let mut curs_pos = 0;

    nodelay(stdscr(), true);

    loop {
        getmaxyx(stdscr(), &mut height, &mut width);

        mvaddstr(1, 0, "-".repeat(width as usize).as_str())?;
        mvaddstr(0, 0, &query)?;

        let key = getch();

        if key == -1 && !update {
            std::thread::sleep(std::time::Duration::from_millis(10));
            continue;
        }

        update = false;

        clear();

        if !searching {
            curs_set(CURSOR_VISIBILITY::CURSOR_INVISIBLE);
            // Toggle search status
            match key {
                9 => {
                    searching = true;
                }

                val if val == 'q' as i32 => {
                    break;
                }

                10 | KEY_ENTER => {
                    if results.is_empty() { continue; }
                
                    run_command("mkdir -p vidimg_yt_dlp".to_string());
                
                    let (_title, url) = &results[curs_pos];

                    clear();
                    mvaddstr(0, 0, "Downloading video...")?;
                    refresh();

                    run_command(format!("yt-dlp -f \"ba+bv/b\" -o \"vidimg_yt_dlp/yt.%(ext)s\" \"{}\"", url));
                
                    if let Some(path) = find_downloaded_file() {
                        play_video(&path, false)?;
                    } else {
                        println!("Error: Download failed or file not found.");
                        std::thread::sleep(std::time::Duration::from_secs(2));
                    }
                
                    run_command("rm -rf vidimg_yt_dlp".to_string());
                
                    refresh(); 
                }

                _ => {}
            }

            if !results.is_empty() {
                match key {
                    KEY_DOWN => {
                        curs_pos = (curs_pos + 1) % results.len();
                    }
                    KEY_UP => {
                        if curs_pos == 0 {
                            curs_pos = results.len() - 1; // Wrap to bottom
                        } else {
                            curs_pos -= 1;
                        }
                    }
                    _ => {}
                }
            }
        }
        else {
            curs_set(CURSOR_VISIBILITY::CURSOR_VISIBLE);
            match key {
                KEY_ENTER | 10 => {
                    searching = false;
                    mvaddstr(2, 0, "Loading...")?;
                    refresh();

                    results = load_videos(query.as_str(), 0, 10);

                    update = true;
                }

                KEY_BACKSPACE => {
                    query.pop();
                }

                -1 => {}

                _ => {
                    let c = char::from_u32(key as u32).unwrap();
                    let s: String = c.to_string();
                    let slice: &str = &s;

                    query += slice;
                }
            }
        }

        for (i, (title, _url)) in results.iter().enumerate() {
            mv((i + 2) as i32, 2);
            addstr(&title)?;
            if i == curs_pos {
                mvaddch((i + 2) as i32, 0, '>' as u32);
            }
        }

        refresh();
    }


    Ok(())
}


fn init_curses() {
    let _ = setlocale(ncurses::LcCategory::all, "");
    initscr();
    cbreak();
    noecho();
    keypad(stdscr(), true);
    start_color();
    curs_set(CURSOR_VISIBILITY::CURSOR_INVISIBLE);

    if !has_colors() {
        panic!("Terminal does not support colors");
    }

    use_default_colors();
    for i in 0..16 {
        init_pair((i + 1) as i16, i as i16, -1); 
    }
}


fn main() -> Result<(), Box<dyn std::error::Error>> {
    init_curses();

    let argv: Vec<String> = env::args().collect();
    if argv.len() < 2 {
        println!("Invalid amount of arguments. At least one required.");
        endwin();
        return Ok(());
    }

    // Variable was already called "unicode". Now it just signifies the second character set 
    let mut unicode = false;
    let mut youtube = false;

    let mut path = String::new();

    for arg in std::env::args().skip(1) {
        match arg.as_str() {
            "-2" | "--set2" => unicode = true,
            "-yt" | "--youtube" => youtube = true,
            _ => path = arg
        }
    }

    if !youtube {
        match get_type(path.as_str()) {
            MediaType::Image | MediaType::Folder => {
                image_roll(path.as_str(), unicode)?;
            }
            MediaType::Video => {
                play_video(path.as_str(), unicode)?;
            }
            _ => {
                mvaddstr(0, 0, "Format not supported or path missing!")?;
                getch();
            }
        };
    }
    else {
        youtube_ui()?;
    }

    endwin();
    Ok(())
}
