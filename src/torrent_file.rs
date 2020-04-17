extern crate bencode;
extern crate base64;

use std::collections::BTreeMap;
use std::convert::TryInto;


use bencode::{FromBencode, Bencode};
use bencode::util::ByteString;
use crate::utils::TorrentError;

type Hash = [u8; 20];


#[derive(Debug, Clone)]
pub struct TorrentFile{
    pub path: String,
    pub size_bytes: usize
}

#[derive(Debug, Clone)]
pub struct TorrentInfo{
    pub files: Vec<TorrentFile>,
    pub announce_url: String,
    pub alternative_announce_url: Vec<String>,
    pub creation_date: u64,
    pub comment: String,
    pub creator: String,
    pub hashes: Vec<Hash>,
    pub byte_size: u64,
    pub piece_byte_size: u64,
    pub num_pieces: usize,
    pub info_hash: [u8; 20]
}

impl TorrentInfo{

    fn process_single_file(file_info: &BTreeMap<ByteString, Bencode>) -> Result<Vec<TorrentFile>, TorrentError>{
        let path: String = match file_info.get(&ByteString::from_str("name")){
            Some(e) => {match FromBencode::from_bencode(e){Ok(b) => b, _ => return Err(TorrentError::new(format!("Unable to find the name of a file")))}}
            _ => return Err(TorrentError::new(String::from("Unable to find name property for file")))
        };

        let size_bytes: usize = match file_info.get(&ByteString::from_str("length")){
            Some(e) => {match FromBencode::from_bencode(e){Ok(b) => b, _ => return Err(TorrentError::new(format!("Unable to find the length of a file")))}}
            _ => return Err(TorrentError::new(String::from("Unable to find name property for file")))
        };

        let mut ret: Vec<TorrentFile> = Vec::new();
        ret.push(TorrentFile{
            path,
            size_bytes
        });

        Ok(ret)
    }

    fn process_multi_file(file_info: &BTreeMap<ByteString, Bencode>) -> Result<Vec<TorrentFile>, TorrentError>{
        let mut ret: Vec<TorrentFile> = Vec::new();
        let top_dir: String = match file_info.get(&ByteString::from_str("name")){
            Some(e) => {match FromBencode::from_bencode(e){Ok(b) => b, _ => return Err(TorrentError::new(String::from("Unable to parse the directory name of a multi file torrent")))}}
            _ => return Err(TorrentError::new(String::from("Unable to find the top directory name for the multi file spec")))
        };
        
        let files_unspecified: &Bencode = match file_info.get(&ByteString::from_str("files")){
            Some(e) => e,
            _ => return Err(TorrentError::new(String::from("Unable to find the file entries for the multi file spec")))
        };

        let files: &Vec<Bencode> = match &files_unspecified{
            &Bencode::List(e) => e,
            _ => return Err(TorrentError::new(String::from("Invalid file list for multi file spec")))
        };

        for bencode_entry in files{
            let file_dict: &BTreeMap<ByteString, Bencode> = match bencode_entry{
                Bencode::Dict(e) => e,
                _ => return Err(TorrentError::new(String::from("Found malformed file info directory")))
            };

            let size_bytes: usize = match file_dict.get(&ByteString::from_str("length")){
                Some(e) => {match FromBencode::from_bencode(e){Ok(e) => e, _ => return Err(TorrentError::new(String::from("Malformed file entry, invalid length")))}}
                _ => return Err(TorrentError::new(String::from("Could not find length variable for file")))
            };

            let path_vec: Vec<String> = match file_dict.get(&ByteString::from_str("path")){
                Some(e) => {match FromBencode::from_bencode(e){Ok(e) => e, _ => return Err(TorrentError::new(String::from("Malformed file entry, invalid path")))}}
                _ => return Err(TorrentError::new(String::from("Could not find path variable for file")))
            };

            let path: String = format!("{}/{}", top_dir, path_vec.join("/"));
            
            ret.push(TorrentFile{
                path,
                size_bytes
            })
        };

        Ok(ret)
    }

    fn process_files(file_info: &BTreeMap<ByteString, Bencode>) -> Result<Vec<TorrentFile>, TorrentError>{
        if file_info.contains_key(&ByteString::from_str("files")){
            return TorrentInfo::process_multi_file(file_info);
        }
        else{
            return TorrentInfo::process_single_file(file_info);
        }
    }

    pub fn from_buffer(buf: Vec<u8>) -> Result<TorrentInfo, TorrentError>{
        let bencoding: bencode::Bencode = match bencode::from_vec(buf){
            Ok(e) => e,
            Err(_e) => return Err(TorrentError::new(String::from(format!("Unable to parse torrent file"))))
        };

        let torrent_info: BTreeMap<ByteString, Bencode> = match bencoding{
            Bencode::Dict(m) => m,
            _ => return Err(TorrentError::new(String::from("Invalid torrent file, no top level dict")))
        };

        let mut ret: TorrentInfo = TorrentInfo{
            files: Vec::new(),
            announce_url: String::from(""),
            alternative_announce_url: Vec::new(),
            creation_date: 0,
            comment: String::from(""),
            creator: String::from(""),
            hashes: Vec::new(),
            byte_size: 0,
            piece_byte_size: 0,
            num_pieces: 0,
            info_hash: [0; 20]
        };

        ret.announce_url = match torrent_info.get(&ByteString::from_str("announce")){
            Some(a) => {match FromBencode::from_bencode(a){
                Ok(b) => b,
                _ => return Err(TorrentError::new(String::from("Invalid announce url")))
            }},
            _ => return Err(TorrentError::new(String::from("Unable to find announce_url")))
        };

        ret.comment = match torrent_info.get(&ByteString::from_str("comment")){
            Some(a) => {match FromBencode::from_bencode(a){
                Ok(b) => b,
                _ => String::from("<No Comment>")
            }},
            _ => String::from("<No Comment>")
        };

        ret.creator = match torrent_info.get(&ByteString::from_str("created by")){
            Some(a) => {match FromBencode::from_bencode(a){
                Ok(b) => b,
                _ => String::from("<Unknown Author>")
            }},
            _ => String::from("<Unknown Author>")
        };

        let announce_list: Vec<String> = match torrent_info.get(&ByteString::from_str("announce-list")){
            Some(a) => {match FromBencode::from_bencode(a){
                Ok(b) => b,
                _ => Vec::new()
            }},
            _ => Vec::new()
        };

        // TODO: remove duplicates from the announce url list and the announce url

        ret.alternative_announce_url = announce_list;

        ret.creation_date = match torrent_info.get(&ByteString::from_str("creation date")){
            Some(a) => {match FromBencode::from_bencode(a){
                Ok(b) => b,
                _ => 0
            }},
            _ => 0
        };

        let file_info: &BTreeMap<ByteString, Bencode> = match torrent_info.get(&ByteString::from_str("info")){
            Some(b) => {match b{
                &Bencode::Dict(ref b) => b,
                _ => return Err(TorrentError::new(String::from("Torrent info is not of type dictionary")))
            }},

            _ => return Err(TorrentError::new(String::from("Unable to find torrent file info")))
        };

        let mut info_bytes= Bencode::Dict(file_info.clone()).to_bytes().unwrap();
        let mut hasher = sha1::Sha1::new();
        hasher.update(&mut info_bytes);
        ret.info_hash = hasher.digest().bytes();

        ret.piece_byte_size = match file_info.get(&ByteString::from_str("piece length")){
            Some(e) => {match FromBencode::from_bencode(e){Ok(b) => b, _ => return Err(TorrentError::new(format!("Unable to find the piece length of a file")))}}
            _ => return Err(TorrentError::new(String::from("Unable to find piece length property for file")))
        };

        let pieces: &Vec<u8> = match file_info.get(&ByteString::from_str("pieces")){
            Some(e) => {match &e {
                &Bencode::ByteString(b) => b,
                _ => return Err(TorrentError::new(String::from("Unable to get the hashes as a byte array")))
            }},
            _ => return Err(TorrentError::new(String::from("Unable to find pieces property for file")))
        };

        if (pieces.len() % 20) != 0 {
            return Err(TorrentError::new(String::from("Unable to find pieces property for file")));
        }
        else{
            for i in 0..(pieces.len()/20){
                ret.hashes.push(match (&pieces[(i*20)..((i+1)*20)]).try_into(){
                        Ok(e) => e,
                        _ => return Err(TorrentError::new(String::from("Invalid slice length")))
                    });
            }
        }

        ret.num_pieces = pieces.len() /20;
        ret.byte_size = ret.piece_byte_size * ret.num_pieces as u64;

        ret.files = match TorrentInfo::process_files(file_info){
            Ok(e) => e,
            Err(e) => return Err(e)
        };

        Ok(ret)
    }

    pub fn from_filename(name: String) -> Result<TorrentInfo, TorrentError>{
        let buf = match std::fs::read(name){
            Ok(e) => e,
            _ => return Err(TorrentError::new(String::from("Could not read file")))
        };

        let torrent_info: TorrentInfo = match TorrentInfo::from_buffer(buf){
            Ok(e) => e,
            Err(e) => return Err(e)
        };

        Ok(torrent_info)
    }
}
