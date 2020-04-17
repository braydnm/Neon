extern crate clokwerk;
extern crate rand;

use crate::torrent::Torrent;

use std::sync::{Mutex, Arc};
use std::fmt::{Debug, Formatter};

use self::clokwerk::{ScheduleHandle};

use percent_encoding::percent_encode_byte;
use std::collections::BTreeMap;
use bencode::{Bencode, FromBencode};
use bencode::util::ByteString;

use crate::peers::Peer;
use crate::utils::{TorrentError, u32_to_bytes, u64_to_bytes, bytes_to_u32, bytes_to_u64, u16_to_bytes, i64_to_bytes, i32_to_bytes, bytes_to_i32, bytes_to_i64, bytes_to_u16};
use self::rand::{Rng, RngCore};
use minreq::Method::Connect;
use std::net::{UdpSocket, ToSocketAddrs, Ipv4Addr};
use std::time::Duration;
use dns_lookup::lookup_host;
use std::convert::TryInto;

type TorrentMutex = Arc<Mutex<Torrent>>;

pub struct Tracker{
    pub torrent_mutex: Option<TorrentMutex>,
    pub schedule_handle: Option<ScheduleHandle>,
    pub interval: usize
}

fn encode_param(data: &[u8]) -> String {
    let mut resp = String::new();
    for byte in data {
        let c = char::from(*byte);
        if (*byte > 0x20 && *byte < 0x7E) && (c.is_numeric() || c.is_alphabetic() || c == '-') {
            resp.push(c);
        } else {
            resp += percent_encode_byte(*byte);
        }
    }
    resp
}

fn transaction_id() -> i32{
    rand::thread_rng().gen()
}

const UDP_DEFAULT_CONNECTION_ID: i64 = 0x41727101980;
const CONNECT: i32 = 0;
const ANNOUNCE: i32 = 1;
const SCRAP: i32 = 2;
const ERROR: i32 = 3;

impl Tracker{

    // TODO: Implement UDP tracker protocol

    fn udp_write(socket: &mut UdpSocket, msg: Vec<u8>) -> Result<(), TorrentError>{
        for i in 1..3{
            socket.set_write_timeout(Some(Duration::from_secs(2u64.pow(i) * 15))).unwrap();
            let size_sent = match socket.send(&msg){
                Ok(e) => e,
                Err(e) => {println!("{:#?}", e);continue}
            };

            if size_sent != msg.len(){
                return Err(TorrentError::new("Unable to write entire message to the UDP client".to_string()));
            }
            else{
                return Ok(())
            }
        }

        return Err(TorrentError::new("Writing to UDP client timed out".to_string()))
    }

    fn announce_udp(torrent_info: &mut Torrent) -> Result<Vec<Box<Peer>>, TorrentError>{
        // create a UDP socket with the tracker
        let mut socket = match UdpSocket::bind("0.0.0.0:0"){
            Ok(e) => e,
            Err(e) => return Err(TorrentError::new(e.to_string()))
        };


        let url_parsed = urlparse::urlparse(torrent_info.info.announce_url.clone());
        let udp_ip = lookup_host(&url_parsed.hostname.unwrap()).unwrap()[0];

        socket.connect((udp_ip, url_parsed.port.unwrap())).expect("Could not connect");
        socket.set_read_timeout(Some(Duration::from_secs(15)));
        // build our connection method with the following form
        // Bytes 0-8: Connection ID, use default for initial connection request
        // Bytes 8-12: Action type
        // Bytes 12-16: Transaction ID to verify response

        let connect_transaction_id = transaction_id();
        let mut connect_msg: Vec<u8> = Vec::new();
        connect_msg.extend(i64_to_bytes(UDP_DEFAULT_CONNECTION_ID));
        connect_msg.extend(i32_to_bytes(CONNECT));
        connect_msg.extend(i32_to_bytes(connect_transaction_id));

        // send our connect request
        match Tracker::udp_write(&mut socket, connect_msg){
            Ok(_) => (),
            Err(e) => return Err(e)
        };

        let mut connect_resp: [u8; 16] = [0; 16];
        let connect_resp_size = match socket.recv(&mut connect_resp){
            Ok(e) => e,
            Err(e) => return Err(TorrentError::new(e.to_string()))
        };

        if connect_resp_size != 16{
            return Err(TorrentError::new("Received connect response that is too long".to_string()));
        }

        // verify the transaction ID and action returned are what we expect
        if bytes_to_i32(&connect_resp[4..8]) != connect_transaction_id{
            return Err(TorrentError::new("UDP response with incorrect transaction id".to_string()));
        }

        if bytes_to_i32(&connect_resp[0..4]) != 0{
            return Err(TorrentError::new("Incorrect action from UDP response".to_string()));
        }

        // set our connection id returned by the server for the rest of the connection with the tracker
        let connection_id = bytes_to_i64(&connect_resp[8..]);

        let mut announce_msg: Vec<u8> = Vec::new();
        let announce_transaction_id = transaction_id();
        announce_msg.extend(i64_to_bytes(connection_id));
        // action = 1 for announce
        announce_msg.extend(i32_to_bytes(1));
        announce_msg.extend(i32_to_bytes(announce_transaction_id));
        announce_msg.extend(&torrent_info.info.info_hash);
        announce_msg.extend(torrent_info.id.clone().into_bytes());
        // 0 bytes downloaded
        announce_msg.extend(i64_to_bytes(0));
        // all the bytes left
        announce_msg.extend(i64_to_bytes(torrent_info.info.byte_size as i64));
        // 0 bytes uploaded
        announce_msg.extend(i64_to_bytes(0));
        // Event type is 2 = Download
        announce_msg.extend(i32_to_bytes(2));
        // IP Address = 0
        announce_msg.extend(u32_to_bytes(0));
        // Random key
        announce_msg.extend(u32_to_bytes(rand::thread_rng().gen()));
        // Number of peers requested = -1 for all
        announce_msg.extend(i32_to_bytes(200));
        // Port, doesn't really matter because we can't seed yet but ok
        // speaking of which, TODO: implement seeding functionality
        announce_msg.extend(u16_to_bytes(6800));
        // extensions are all 0
        announce_msg.extend(u16_to_bytes(0));

        Tracker::udp_write(&mut socket, announce_msg).expect("Unable to write announce request");

        // read the first 20 bytes to get info on the number of peers we are receiving
        let mut announce_info = [0u8; 20 + 2000 * 6];
        let announce_resp_size = match socket.recv(&mut announce_info){
            Ok(e) => e,
            Err(e) => return Err(TorrentError::new(e.to_string()))
        };

        // if there is less than one peer panic
        if announce_resp_size < 26 || (announce_resp_size - 20) % 6 != 0{
            return Err(TorrentError::new("UDP tracker returned invalid announce DGRAM header size".to_string()))
        }

        if bytes_to_i32(&announce_info[0..4]) != 1{
            return Err(TorrentError::new("Invalid action, expected announce".to_string()));
        }

        if bytes_to_i32(&announce_info[4..8]) != announce_transaction_id{
            return Err(TorrentError::new("Invalid transaction ID returned after announce".to_string()));
        }

        // for now ignore the interval because we can't really handle re-announce currently :(
        let num_seeders = bytes_to_i32(&announce_info[16..20]);
        let mut peers: Vec<Box<Peer>> = Vec::new();

        for i in 0..((announce_resp_size-20)/6){
            let start = 20 + (i * 6);
            let end = 20 + ((i+1) * 6);
            let ip_addr = bytes_to_u32(&announce_info[start..end-2]);
            let port = bytes_to_u16(&announce_info[start+4..end]);

            let ip = Ipv4Addr::from(ip_addr).to_string();

            peers.push(Box::new(Peer::new(
                ip,
                port,
                None,
                torrent_info.id.clone(),
                torrent_info.info.info_hash,
                torrent_info.info.num_pieces,
                torrent_info.info.piece_byte_size as usize
            )))
        }

        Ok(peers)
    }


    // TODO: Implement DHL tracker protocol

    fn announce_http(torrent_info: &mut Torrent) -> Result<Vec<Box<Peer>>, TorrentError>{
        let params = [("info_hash", encode_param(&torrent_info.info.info_hash)),
            ("peer_id", encode_param(&torrent_info.id.as_bytes())),
            ("port", torrent_info.port.to_string()), ("uploaded", torrent_info.uploaded.to_string()),
            ("downloaded", torrent_info.downloaded.to_string()), ("compact", String::from("1")),
            ("left", (torrent_info.info.byte_size - torrent_info.downloaded as u64).to_string()),
            ("event", "started".to_string()), ("numwant", "100".to_string())];

        let mut param_vec = Vec::new();
        for (key, val) in params.iter(){
            param_vec.push(format!("{}={}", key, val));
        }

        let url = format!("{}?{}", torrent_info.info.announce_url, param_vec.join("&"));

        let response = match minreq::get(url).send() {
            Ok(e) => e.into_bytes(),
            _ => return Err(TorrentError::new("Unable to get a response from the tracker".to_string()))
        };


        let bencodeing= match bencode::from_vec(response){
            Ok(e) => e,
            Err(e) => return Err(TorrentError::new(e.msg))
        };

        let peer_dict: BTreeMap<ByteString, Bencode> = match bencodeing{
            Bencode::Dict(e) => e,
            _ => return Err(TorrentError::new("Invalid bencode".to_string()))
        };

        let _interval: usize = match peer_dict.get(&ByteString::from_str("interval")){
            Some(e) => {match FromBencode::from_bencode(e){Ok(b)=>b, _=>1000*1000}},
            _ => 1000 * 1000
        };

        let byte_peers: &Vec<u8> = match peer_dict.get(&ByteString::from_str("peers")){
            Some(e) => {match e{Bencode::ByteString(b) => b, _ => return Err(TorrentError::new("Malformed peer list".to_string()))}},
            _ => return Err(TorrentError::new("Could not find peer list".to_string()))
        };

        let mut peers: Vec<Box<Peer>> = Vec::new();

        if byte_peers.len() % 6 != 0{
            return Err(TorrentError::new("Malformed peer length".to_string()))
        }

        for i in 0..byte_peers.len() / 6{
            let peer_slice = &byte_peers[i * 6 .. (i+1)*6];
            let ip: String = format!("{}.{}.{}.{}", peer_slice[0].to_string(), peer_slice[1].to_string(), peer_slice[2].to_string(), peer_slice[3].to_string());
            let port: u16 = ((peer_slice[4] as u16) << 8)+ peer_slice[5] as u16;
            peers.push(Box::new(Peer::new(
                ip,
                port,
                None,
                torrent_info.id.clone(),
                torrent_info.info.info_hash,
                torrent_info.info.num_pieces,
                torrent_info.info.piece_byte_size as usize
            )));
        }

        Ok(peers)
    }

    pub fn announce(torrent_info: &mut Torrent) -> Result<Vec<Box<Peer>>, TorrentError>{

        if torrent_info.info.announce_url.starts_with("http"){
            return Tracker::announce_http(torrent_info);
        }
        else if torrent_info.info.announce_url.starts_with("udp"){
            return Tracker::announce_udp(torrent_info);
        }
        else{
            return Err(TorrentError::new("Unsupported tracker protocol".to_string()));
        }
    }
}

impl Debug for Tracker{
    fn fmt(&self, _f: &mut Formatter<'_>) -> Result<(),std::fmt::Error> {
        unimplemented!()
    }
}

impl Clone for Tracker{
    fn clone(&self) -> Self {
        Tracker{
            torrent_mutex: self.torrent_mutex.clone(),
            schedule_handle: None,
            interval: self.interval
        }
    }

    fn clone_from(&mut self, _source: &Self) {
        unimplemented!()
    }
}