with import <nixpkgs> {};

mkShell {
  buildInputs = [
    rustc
    cargo
    ncurses
    pkg-config
    ffmpeg
    yt-dlp
  ];
}
