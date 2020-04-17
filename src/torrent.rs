extern crate rand;

use rand::Rng;
use rand::distributions::Alphanumeric;

use crate::peers::Peer;
use crate::tracker::Tracker;
use crate::torrent_file::{TorrentInfo};
use crate::utils::{TorrentChannel, TorrentEventType, TorrentEvent};

use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::io;
use std::io::{Write};

use colored::Colorize;

use bit_vec::BitVec;

use crossbeam_queue::ArrayQueue;
use std::fs::File;
use crossbeam_channel::{unbounded, Sender, Receiver, bounded};

const ID_BEGIN: &str = "-NE001-";

macro_rules! thread_println {
    ($( $args:expr ),*) => {
        writeln!(&mut io::stdout().lock(), $( $args ),* ).expect("Cannot write to stdout");
    }
}

#[derive(Debug)]
pub struct Torrent{
    pub id: String,
    pub port: u16,
    pub downloaded: usize,
    pub uploaded: usize,
    pub info: TorrentInfo,
    work_queue: Arc<ArrayQueue<(u32, u64)>>,
    pub download_events: Option<Receiver<TorrentEvent>>,
    pub peer_thread_handles: Vec<JoinHandle<()>>,
    peer_channel_senders: Vec<Sender<TorrentEvent>>,
    pub torrent_mutex: Option<Arc<Mutex<Torrent>>>,
    data: Vec<u8>,
    bitfield: BitVec
}

fn random_alphanumeric(size: usize) -> String{
    rand::thread_rng().sample_iter(&Alphanumeric).take(size).collect::<String>()
}

impl Torrent{
    pub fn new(info: TorrentInfo) -> Arc<Mutex<Torrent>>{
        let id: String = format!("{}{}", ID_BEGIN, random_alphanumeric(20 - ID_BEGIN.len()));
        let mut ret = Torrent{
            id,
            port: 1881,
            downloaded: 0,
            uploaded: 0,
            info: info.clone(),
            download_events: None,
            peer_thread_handles: Vec::new(),
            peer_channel_senders: Vec::new(),
            torrent_mutex: None,
            bitfield: BitVec::new(),
            work_queue: Arc::new(ArrayQueue::new(info.num_pieces)),
            data: Vec::with_capacity(info.byte_size as usize)
        };

        ret.bitfield.reserve(info.num_pieces);

        let mutex = Arc::new(Mutex::new(ret));
        let mut torrent = &mut *mutex.lock().unwrap();
        torrent.torrent_mutex = Some(mutex.clone());
        mutex.clone()
    }

    pub fn download(&mut self, output_name: &String){
        let (sender, receiver): (Sender<TorrentEvent>, Receiver<TorrentEvent>) = unbounded();
        self.download_events = Some(receiver);
        let mut new_peers: Vec<Box<Peer>> = Tracker::announce(self).unwrap();
        println!("[{}] Announced to tracker - received {} peers", "*".green(), new_peers.len());

        for i in 0..self.info.num_pieces{
            let size;
            if (i+1) as u64 * self.info.piece_byte_size > self.info.byte_size{
                size = self.info.byte_size - (i as u64 * self.info.piece_byte_size);
            }
            else{
                size = self.info.piece_byte_size;
            }
            self.work_queue.push((i as u32, size)).expect("Cannot add work to queue");
        }

        let output_data: Vec<u8> = vec![0; self.info.byte_size as usize];
        let output_arc = Arc::new(Mutex::new(output_data));

        for _ in 0..new_peers.len() {
            let (individual_sender, receiver): (Sender<TorrentEvent>, Receiver<TorrentEvent>) = bounded(3);
            self.peer_channel_senders.push(individual_sender);
            let peer = new_peers.pop().unwrap();
            let channel: TorrentChannel<TorrentEvent> = TorrentChannel::new(self.work_queue.clone(), sender.clone(), receiver);
            self.peer_thread_handles.push(Peer::start_download(*peer, channel, output_arc.clone()));
        }

        let downloaded = self.download_events.as_ref().unwrap();
        let mut pieces_received = 0;
        let mut num_peers = 0;

        let mut hasher = sha1::Sha1::new();
        while pieces_received != self.info.num_pieces{
            let work_done = downloaded.try_recv();
            if work_done.is_ok(){
                let event = work_done.unwrap();
                if event.msg_type == TorrentEventType::Downloaded {
                    hasher.reset();
                    let index = event.index_downloaded.unwrap();
                    let start = index as u64 * self.info.piece_byte_size;
                    let end = self.info.byte_size.min((index + 1) as u64 * self.info.piece_byte_size);
                    hasher.update(&output_arc.lock().unwrap()[start as usize .. end as usize]);
                    if hasher.digest().bytes() != self.info.hashes[index as usize]{
                        thread_println!("[{}] Received invalid piece at index {}", "X".red(), index);
                        let size;

                        if index == (self.info.num_pieces - 1) as u32 {
                            size = self.info.byte_size - (index * self.info.piece_byte_size as u32) as u64;
                        }
                        else{
                            size = self.info.piece_byte_size;
                        }

                        self.work_queue.push((index, size)).expect("Cannot add work to queue");
                        continue;
                    }

                    pieces_received += 1;
                    thread_println!("[{}] ({:.2}%) Downloaded piece {} from {} peers", "*".green(), (pieces_received as f32 / self.info.num_pieces as f32) * 100.0, index, num_peers);
                }
                else if event.msg_type == TorrentEventType::Active {
                    num_peers += 1;
                }
                else if event.msg_type == TorrentEventType::Close{
                    num_peers -= 1;
                }
            }
        }

        thread_println!("Completed download - joining threads");
        let file = File::create(output_name);
        file.unwrap().write_all(&output_arc.lock().unwrap()).expect("Unable to write to file");
    }

    pub fn join(&mut self){
        for _ in 0..self.peer_thread_handles.len(){
            self.peer_thread_handles.pop().unwrap().join().expect("Unable to join thread");
        }
    }
}