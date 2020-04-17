use crossbeam_queue::ArrayQueue;

use colored::Colorize;
use std::sync::Arc;
use std::io::Cursor;
use byteorder::{BigEndian, ReadBytesExt, WriteBytesExt};
use crossbeam_channel::{Sender, Receiver};

pub fn bytes_to_u16(bytes: &[u8]) -> u16 {
    let mut buf = Cursor::new(&bytes);
    buf.read_u16::<BigEndian>().unwrap()
}

pub fn u16_to_bytes(integer: u16) -> Vec<u8> {
    let mut bytes = vec![];
    bytes.write_u16::<BigEndian>(integer).unwrap();
    bytes
}

pub fn bytes_to_u32(bytes: &[u8]) -> u32 {
    let mut buf = Cursor::new(&bytes);
    buf.read_u32::<BigEndian>().unwrap()
}

pub fn u32_to_bytes(integer: u32) -> Vec<u8> {
    let mut bytes = vec![];
    bytes.write_u32::<BigEndian>(integer).unwrap();
    bytes
}

pub fn bytes_to_u64(bytes: &[u8]) -> u64 {
    let mut buf = Cursor::new(&bytes);
    buf.read_u64::<BigEndian>().unwrap()
}

pub fn u64_to_bytes(integer: u64) -> Vec<u8> {
    let mut bytes = vec![];
    bytes.write_u64::<BigEndian>(integer).unwrap();
    bytes
}

pub fn bytes_to_i16(bytes: &[u8]) -> u16 {
    let mut buf = Cursor::new(&bytes);
    buf.read_u16::<BigEndian>().unwrap()
}

pub fn i16_to_bytes(integer: i16) -> Vec<u8> {
    let mut bytes = vec![];
    bytes.write_i16::<BigEndian>(integer).unwrap();
    bytes
}

pub fn bytes_to_i32(bytes: &[u8]) -> i32 {
    let mut buf = Cursor::new(&bytes);
    buf.read_i32::<BigEndian>().unwrap()
}

pub fn i32_to_bytes(integer: i32) -> Vec<u8> {
    let mut bytes = vec![];
    bytes.write_i32::<BigEndian>(integer).unwrap();
    bytes
}

pub fn bytes_to_i64(bytes: &[u8]) -> i64 {
    let mut buf = Cursor::new(&bytes);
    buf.read_i64::<BigEndian>().unwrap()
}

pub fn i64_to_bytes(integer: i64) -> Vec<u8> {
    let mut bytes = vec![];
    bytes.write_i64::<BigEndian>(integer).unwrap();
    bytes
}

#[derive(Debug, Clone)]
pub struct TorrentError{
    pub details: String
}

impl TorrentError{
    pub fn new(msg: String) -> TorrentError{
        TorrentError{
            details: format!("[{}] ", "X".red()) + &msg
        }
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum TorrentEventType{
    Close,
    Active,
    Downloaded,
    Request,
    Cancel,
    Exit
}

#[derive(Debug, Clone)]
pub struct TorrentChannel<T>{
    pub sender: Sender<T>,
    pub receiver: Receiver<T>,
    pub work_queue: Arc<ArrayQueue<(u32, u64)>>
}

impl<T> TorrentChannel<T>{
    pub fn new(queue: Arc<ArrayQueue<(u32, u64)>>, sender: Sender<T>, receiver: Receiver<T>) -> TorrentChannel<T>{
        TorrentChannel{
            sender,
            work_queue: queue,
            receiver
        }
    }

    pub fn send(&mut self, data: T) -> Result<(), TorrentError>{
        if self.sender.send(data).is_err(){
            return Err(TorrentError::new(format!("Unable to send message")));
        }

        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct TorrentPiece{
    pub index: usize,
    pub data: Arc<[u8]>
}

#[derive(Debug, Clone)]
pub struct TorrentEvent{
    pub msg_type: TorrentEventType,
    pub index_downloaded: Option<u32>
}

impl TorrentEvent{
    pub fn new(msg_type: TorrentEventType) -> TorrentEvent{
        TorrentEvent{
            msg_type,
            index_downloaded: None
        }
    }

    pub fn with_index(msg_type: TorrentEventType, index: u32) -> TorrentEvent{
        TorrentEvent{
            msg_type,
            index_downloaded: Some(index)
        }
    }
}