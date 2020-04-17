mod torrent_file;
mod peers;
mod torrent;
mod tracker;
mod utils;

use crate::torrent_file::TorrentInfo;
use colored::Colorize;
use std::env;
use crate::tracker::Tracker;

fn main(){

    let arguments: Vec<String> = env::args().collect();

    if arguments.len() < 3{
        eprintln!("Usage: ./neon <torrent name> <output name>")
    }

    let info = TorrentInfo::from_filename(arguments[1].clone()).unwrap();

    println!("Num Pieces: {}, Piece Size: {}, Num Bytes: {}, Good: {}", info.num_pieces, info.piece_byte_size, info.byte_size, info.num_pieces * info.piece_byte_size as usize == info.byte_size as usize);
    println!("Info Hash: {}", base64::encode(info.info_hash));
    println!("[{}] Parsed torrent info", "*".green());
    let torrent = torrent::Torrent::new(info);
    torrent.lock().unwrap().download(&arguments[2]);
}
