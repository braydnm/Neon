# Neon - A BitTorrent Client

Neon is a personal BitTorrent client that was made as a POC and to learn about network protocols and multithreading, all while learning the language Rust



## Install

Instructions to install Rust and Cargo can be found at: https://doc.rust-lang.org/cargo/getting-started/installation.html

After cloning the repo run

```bash
cargo build --release
```

in the directory and the binary can be found in target/release/neon



## Usage

Try downloading [Arch Linux](https://www.archlinux.org/releng/releases/2020.04.01/torrent/)

```bash
./neon archlinux-2020.04.01-x86_64.iso.torrent arch.iso
```

[![asciicast](https://asciinema.org/a/soDRcbjKx3K4BGjhy1Em7W8kC.svg)](https://asciinema.org/a/soDRcbjKx3K4BGjhy1Em7W8kC?speed=3)

*This video is slightly sped up*

# ToDo:

* Add code to upload to peers as well as download
* Multi-file torrents
* Magnet links
* DHT / UDP trackers
* Improve multithreading performance
  * Instead of a thread for every peer, make the socket non-blocking and allocate a thread for every 5-6 peers
* Tracker reannounce
* Better error handling
* General code cleanup