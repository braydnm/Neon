use std::net::{TcpStream, SocketAddr, Shutdown};
use colored::Colorize;
use std::io::{Read, Write};
use std::time::Duration;
use std::{thread, io};
use std::thread::JoinHandle;
use bit_vec::BitVec;
use crate::utils::{TorrentChannel, TorrentError, TorrentEvent, TorrentEventType, u32_to_bytes, bytes_to_u32};
use std::sync::{Arc, Mutex};
use std::convert::TryInto;
use stopwatch::Stopwatch;

const HANDSHAKE_MSG: &str = "BitTorrent protocol";
const CONNECTION_TIMEOUT: f32 = 3.0;
const WRITE_TIMEOUT: f32 = 2.0;
const READ_TIMEOUT: f32 = 15.0;

const MAX_BLOCK_SIZE: u32 = 16384;
const MAX_BACKLOG: u32 = 100;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RequestType {
    Choked = 0,
    Unchoked = 1,
    Interested = 2,
    Uninterested = 3,
    Have = 4,
    Bitfield = 5,
    Request = 6,
    Piece = 7,
    Cancel = 8,
    Unknown = -1
}

impl From<u8> for RequestType{
    fn from(val: u8) -> Self {
        match val{
            0 => RequestType::Choked,
            1 => RequestType::Unchoked,
            2 => RequestType::Interested,
            3 => RequestType::Uninterested,
            4 => RequestType::Have,
            5 => RequestType::Bitfield,
            6 => RequestType::Request,
            7 => RequestType::Piece,
            8 => RequestType::Cancel,
            _ => RequestType::Unknown
        }
    }
}

macro_rules! thread_println {
    ($( $args:expr ),*) => {
        writeln!(&mut io::stdout().lock(), $( $args ),* ).expect("Cannot write to stdout");
    }
}

#[derive(Debug)]
pub struct Peer{
    pub ip_addr: String,
    pub port: u16,
    pub tracker_id: Option<Vec<u8>>,
    can_request: bool,
    is_choked: bool,
    tcp_stream: Option<TcpStream>,
    info_hash: [u8; 20],
    peer_id: String,
    bitfield: BitVec,
    backlog: u32,
    bytes_downloaded: u32,
    downloaded: Vec<u8>,
    target_index: i32,
    requested: u32,
    seen_already: Vec<u32>,
    closed: bool,
    average: (u128, usize),
    download_speed_kb: f32,
    max_backlog: u32,
    is_active: bool
}

impl Peer{
    pub fn new(ip_addr: String, port: u16, tracker_id: Option<Vec<u8>>, my_peer_id: String, info_hash: [u8; 20], num_pieces: usize, piece_size: usize) -> Peer{
        Peer{
            ip_addr,
            port,
            tracker_id,
            is_choked: true,
            can_request: false,
            tcp_stream: None,
            info_hash,
            peer_id: my_peer_id,
            bitfield: BitVec::from_elem(num_pieces, false),
            backlog: 0,
            bytes_downloaded: 0,
            downloaded: vec![0; piece_size],
            target_index: -1,
            requested: 0,
            closed: false,
            seen_already: Vec::new(),
            average: (0, 0),
            download_speed_kb: 0.0,
            max_backlog: 5,
            is_active: false
        }
    }

    fn create_bitfield(payload: &[u8], spare_bits: usize) -> BitVec{
        let len = payload.len();
        let mut bf = BitVec::from_bytes(&payload[0..(len - 1)]);
        let iter = (spare_bits..8)
            .rev()
            .map(|bit| ((1 << bit) & payload[len - 1] as usize) != 0);
        for elem in iter {
            bf.push(elem);
        }
        return bf;
    }

    fn handle_recv_bitfield(&mut self, payload: Vec<u8>) -> Result<RequestType, TorrentError> {
        if payload.len() == 0 {
            return Err(TorrentError::new(format!("Got bitfield of length 0 from {}", self.peer_id)));
        }

        let spare_bits = 8 - (self.bitfield.len() % 8);
        let parsed_bitfield = Peer::create_bitfield(payload.as_slice(), spare_bits);

        if parsed_bitfield.len() == self.bitfield.len() {
            self.bitfield.union(&parsed_bitfield);
            Ok(RequestType::Bitfield)
        }
        else{
            return Err(TorrentError::new(format!("Received bitfield of different length from {}", self.ip_addr)))
        }
    }

    fn handle_update_bitfield(&mut self, payload: Vec<u8>) -> Result<RequestType, TorrentError>{
        let index: u32 = u32::from_be_bytes(payload.as_slice().try_into().unwrap());
        self.bitfield.set(index as usize, true);
        Ok(RequestType::Bitfield)
    }

    fn make_msg(request_type: RequestType, msg: Vec<u8>) -> Vec<u8> {
        let length: u32 = msg.len() as u32 + 1;
        let mut bitstream: Vec<u8> = u32_to_bytes(length);
        bitstream.push(request_type as u8);
        bitstream.extend(msg);
        assert_eq!(bitstream.len(), length as usize + 4);

        bitstream
    }

    fn read_n(&mut self, num_bytes: u64) -> Result<Vec<u8>, TorrentError>{
        let mut ret = vec![];
        let tcp = self.tcp_stream.as_ref().unwrap();

        let mut take = tcp.take(num_bytes);
        let bytes_read = take.read_to_end(&mut ret);
        match bytes_read{
            Ok(e) => {
                if (e as u64) == num_bytes{
                    Ok(ret)
                }
                else{
                    Err(TorrentError::new(format!("Couldn't read all the bytes from {}", self.ip_addr)))
                }
            },
            Err(e) => Err(TorrentError::new(format!("Error reading from {}, {}", self.ip_addr, e.to_string())))
        }
    }

    fn read_msg(&mut self) -> Result<(RequestType, Vec<u8>), TorrentError>{
        if !self.can_request || self.tcp_stream.is_none(){
            return Err(TorrentError::new(format!("Peer {} cannot make requests or read requests", self.ip_addr.blue())));
        }

       let length = match self.read_n(4){
            Ok(e) => bytes_to_u32(&e),
           _ => 0
        };

        if length > 0{
             let message = match self.read_n(length.into()){
                Ok(e) => e,
                 Err(e) => return Err(e)
            };

            Ok((RequestType::from(message[0]), Vec::from(&message[1..])))
        }
        else{
            return Err(TorrentError::new("Timeout reading stream".to_string()));
        }
    }

    fn write_msg(&mut self, msg: Vec<u8>) -> Result<(), TorrentError>{
        match self.tcp_stream.as_ref().unwrap().write_all(msg.as_ref()){
            Ok(e) => e,
            _ => return Err(TorrentError::new(format!("Peer {}: Unable to write to peer", self.ip_addr)))
        };


        Ok(())
    }

    fn initiate_conn(&mut self, channel:&mut TorrentChannel<TorrentEvent>) -> Result<(), TorrentError>{
        if self.can_request || self.tcp_stream.is_some(){
            return Err(TorrentError::new(format!("Peer {} connection is already created", self.ip_addr.blue())));
        }

        let tcp_stream = match TcpStream::connect_timeout(&SocketAddr::new(self.ip_addr.parse().unwrap(), self.port), Duration::from_secs_f32(CONNECTION_TIMEOUT)){
            Ok(e) => e,
            _ => return Err(TorrentError::new(format!("Could not intiate connection with {}:{}", self.ip_addr, self.port)))
        };
        if tcp_stream.set_write_timeout(Some(Duration::from_secs_f32(WRITE_TIMEOUT))).is_err(){
            return Err(self.close(channel, "Cannot set write timeout on socket".to_string()))
        }

        if tcp_stream.set_read_timeout(Some(Duration::from_secs_f32(READ_TIMEOUT))).is_err(){
            return Err(self.close(channel, "Cannot set read timeout on socket".to_string()))
        }

        if tcp_stream.set_nonblocking(false).is_err(){
            return Err(self.close(channel, "Cannot set socket to be non-blocking".to_string()))
        }


        self.tcp_stream = Some(tcp_stream);
        let mut handshake_msg: Vec<u8> = format!("\x13{}\x00\x00\x00\x00\x00\x00\x00\x00", HANDSHAKE_MSG).into_bytes();
        handshake_msg.extend_from_slice(&mut self.info_hash);
        handshake_msg.extend_from_slice(self.peer_id.as_bytes());
        if self.write_msg(handshake_msg).is_err(){
            return Err(self.close(channel, "Cannot send handshake over socket".to_string()))
        }

        let handshake_len = match self.read_n(1){
            Ok(e) => e[0],
            Err(e) => return Err(e)
        };
        let handshake_str = self.read_n(handshake_len as u64).unwrap();
        let _reserved = self.read_n(8).unwrap();
        let info_hash = self.read_n(20).unwrap();
        let _peer_id = self.read_n(20).unwrap();

        if info_hash != self.info_hash || String::from_utf8(handshake_str).unwrap() != String::from(HANDSHAKE_MSG){
            self.can_request = false;
        }
        else{
            self.can_request = true;
        }

        Ok(())
    }

    fn close(&mut self, channel: &mut TorrentChannel<TorrentEvent>, msg: String) -> TorrentError{

        if self.closed{
            return TorrentError::new(msg);
        }

        self.closed = true;
        if self.is_active {
            if channel.send(TorrentEvent::new(TorrentEventType::Close)).is_err() {
                eprintln!("Peer manager thinks we are still active, this is not good")
            }
        }

        return TorrentError::new(msg);
    }

    fn recv_and_handle_msg(&mut self, channel: &mut TorrentChannel<TorrentEvent>) -> Result<RequestType, TorrentError>{
        let timer = Stopwatch::start_new();
        let msg_result = self.read_msg();
        let how_long = timer.elapsed();

        self.average.0 = (self.average.0 * self.average.1 as u128) + how_long.as_millis();
        self.average.0 /= self.average.1 as u128 + 1;
        self.average.1 += 1;

        let (req_type, payload) = match msg_result{
            Ok(e) => e,
            Err(e) => return Err(self.close(channel, e.details[12..].to_string()))
        };

        match req_type{
            RequestType::Unchoked => {self.is_choked = false; Ok(RequestType::Unchoked)},
            RequestType::Choked => {self.is_choked = true; Ok(RequestType::Choked)},
            RequestType::Bitfield => self.handle_recv_bitfield(payload),
            RequestType::Cancel => return Err(self.close(channel, format!("{} sent cancel request", self.peer_id))),
            RequestType::Interested => {thread_println!("Ignoring interested"); Ok(RequestType::Interested)},
            RequestType::Uninterested => {thread_println!("Ignoring uninterested"); Ok(RequestType::Uninterested)},
            RequestType::Piece => self.receive_piece(payload),
            RequestType::Request => {thread_println!("Ignoring request"); Ok(RequestType::Request)},
            RequestType::Have => self.handle_update_bitfield(payload),
            RequestType::Unknown => return Err(self.close(channel, format!("{} sent unknown request type", self.peer_id)))
        }

    }

    fn receive_piece(&mut self, payload: Vec<u8>) -> Result<RequestType, TorrentError>{
        let index: u32 = u32::from_be_bytes(payload[0..4].try_into().unwrap());

        if index != self.target_index as u32{
            return Err(TorrentError::new(format!("Received invalid index from {}, requested {}, got {}", self.ip_addr, self.target_index, index)));
        }

        let offset: u32 = u32::from_be_bytes(payload[4..8].try_into().unwrap());
        if offset > self.downloaded.capacity() as u32{
            return Err(TorrentError::new(format!("Received data with out of bounds offset from {}", self.peer_id)));
        }

        // thread_println!("Received piece part(Index: {}, Offset: {})", index, offset);

        for i in 0..payload.len() - 8{
            self.downloaded[offset as usize + i] = payload[8+i]
        }

        self.bytes_downloaded += payload.len() as u32 - 8;
        self.backlog -= 1;

        Ok(RequestType::Piece)
    }

    fn request_piece(&mut self, offset: u32, length: u32) -> Result<(), TorrentError>{
        let mut request_bytes: Vec<u8> = Vec::new();
        request_bytes.extend(u32_to_bytes(self.target_index as u32));
        request_bytes.extend(u32_to_bytes(offset));
        request_bytes.extend(u32_to_bytes(length));

        let msg = Peer::make_msg(RequestType::Request, request_bytes);
        // thread_println!("Piece {}, Offset {}, Length: {}. Message Encoded: {:?}", self.target_index, offset, length, msg);
        self.write_msg(msg)
    }

    pub fn start_download(mut peer: Peer, mut channel: TorrentChannel<TorrentEvent>, output: Arc<Mutex<Vec<u8>>>) -> JoinHandle<()>{
        thread::spawn(move ||{
            let init = peer.initiate_conn(&mut channel);
            match init{
               Ok(_e) => thread_println!("[{}] Completed connection with {}", "*".green(), peer.ip_addr),
                Err(e) => {thread_println!("{}", e.details); return;}
           };

            // receive the bitfield from our peer
            let bitfield_rsp = peer.recv_and_handle_msg(&mut channel);
            if bitfield_rsp.is_err(){
                thread_println!("Unable to read bitfield");
                return;
            }
            else if bitfield_rsp.unwrap() != RequestType::Bitfield{
                thread_println!("Expected Bitfield, exiting");
                return;
            }

            // tell our peer we are interested and want to be unchoked
            if peer.write_msg(Peer::make_msg(RequestType::Unchoked, Vec::new())).is_err(){
                thread_println!("{}", peer.close(&mut channel, "Cannot request to be unchoked".to_string()).details);
                return;
            }
            if peer.write_msg(Peer::make_msg(RequestType::Interested, Vec::new())).is_err(){
                thread_println!("{}", peer.close(&mut channel, "Cannot tell peer we are interested".to_string()).details);
                return;
            }

            // attempt to be unchoked, if we are not unchoked then severe the connection
            for _ in 0..2{
                if channel.work_queue.is_empty(){
                    return;
                }

                let resp = peer.read_msg();

                if resp.is_err(){
                    continue;
                }

                if resp.unwrap().0 == RequestType::Unchoked{
                    peer.is_choked = false;
                    break;
                }
            }

            if peer.is_choked{
                thread_println!("{}", peer.close(&mut channel, "Did not get unchoked, exiting".to_string()).details);
                return;
            }
            
            // while there is still work to do
            'job_poll: loop{
                peer.requested = 0;
                peer.bytes_downloaded = 0;
                peer.backlog = 0;

                if peer.is_choked{
                    peer.close(&mut channel, "I am choked".to_string());
                    return;
                }

                let (index, size) = match channel.work_queue.pop(){
                  Ok(e) => e,
                    _ => {thread::sleep(Duration::from_secs_f32(0.5)); continue;}
                };



                if !peer.bitfield[index as usize]{
                    channel.work_queue.push((index, size)).expect("Cannot add work to queue, this torrent will be incomplete");

                    thread_println!("[{}] Cannot satisfy {} from peer {}", "-".yellow(), index, peer.ip_addr);
                    continue;
                }

                let timer = Stopwatch::start_new();

                peer.target_index = index as i32;
                while peer.bytes_downloaded < size as u32{
                    while peer.backlog < peer.max_backlog && peer.requested < size as u32{
                        let size_wanted = MAX_BLOCK_SIZE.min(size as u32 - peer.requested);
                        if peer.request_piece(peer.requested, size_wanted).is_err(){
                            channel.work_queue.push((index, size)).expect("Cannot add work to queue, this torrent will be incomplete");
                            break 'job_poll;
                        }
                        peer.backlog += 1;
                        peer.requested += size_wanted;
                    }
                    match peer.recv_and_handle_msg(&mut channel){
                        Ok(_e) => (),
                        Err(e) => {
                            thread_println!("{}", e.details);
                            channel.work_queue.push((index, size)).expect("Cannot add work to queue, this torrent will be incomplete");
                            break 'job_poll;
                        }
                    };
                    // TODO: Implement the Endgame algorithm instead of this hacky alternative
                    if channel.work_queue.len() < 20 && peer.bytes_downloaded as f32 / timer.elapsed().as_secs_f32() < 60.0{
                        channel.work_queue.push((index, size)).expect("Cannot add work to queue, this torrent will be incomplete");
                        continue;
                    }
                }

                if size as f32 / timer.elapsed().as_secs_f32() > 20.0{
                    peer.max_backlog += 2
                }
                else{
                    peer.max_backlog = MAX_BACKLOG.min(peer.max_backlog / 5 + 18);
                }

                peer.average.0 = 0;
                peer.average.1 = 0;

                let write_scheduler = Stopwatch::start_new();

                let mut output = output.lock().unwrap();
                let piece_size = peer.downloaded.capacity();

                for i in 0..piece_size{
                    output[(index as usize * piece_size) + i] = peer.downloaded[i]
                }

                drop(output);

                channel.send(TorrentEvent::with_index(TorrentEventType::Downloaded, index)).expect("Cannot tell manager we are complete, this torrent will be incomplete");
                if !peer.is_active {
                    if channel.send(TorrentEvent::new(TorrentEventType::Active)).is_err() {
                        thread_println!("{}", peer.close(&mut channel, "Cannot send event to manager".to_string()).details);
                        return;
                    }
                    peer.is_active = true;
                }
            }

            peer.close(&mut channel, "Completed jobs".to_string());
        })
    }
}

#[cfg(test)]
mod message_tests {
    use crate::utils::bytes_to_u32;

    #[test]
    fn test_piece_parse(){
        let mut payload: Vec<u8> = [0, 0, 1, 1, 0, 0, 1, 2, 0, 0, 1, 3, 4, 5].to_vec();
        let index: u32 = bytes_to_u32(&payload[0..4]);
        let offset: u32 = bytes_to_u32(&payload[4..8]);

        assert_eq!(index, 257);
        assert_eq!(offset, 258);
    }

}