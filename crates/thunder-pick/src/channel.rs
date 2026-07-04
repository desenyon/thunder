use std::io::{self, BufRead};
use std::sync::mpsc::Receiver;

/// BufRead adapter that pulls newline-delimited items from a channel.
pub struct ChannelReader {
    rx: Receiver<String>,
    buffer: Vec<u8>,
    pos: usize,
    finished: bool,
}

impl ChannelReader {
    pub fn new(rx: Receiver<String>) -> Self {
        Self {
            rx,
            buffer: Vec::new(),
            pos: 0,
            finished: false,
        }
    }
}

impl io::Read for ChannelReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.pos >= self.buffer.len() && !self.finished {
            match self.rx.recv() {
                Ok(line) => {
                    self.buffer = line.into_bytes();
                    self.buffer.push(b'\n');
                    self.pos = 0;
                }
                Err(_) => self.finished = true,
            }
        }
        if self.pos >= self.buffer.len() {
            return Ok(0);
        }
        let n = buf.len().min(self.buffer.len() - self.pos);
        buf[..n].copy_from_slice(&self.buffer[self.pos..self.pos + n]);
        self.pos += n;
        Ok(n)
    }
}

impl BufRead for ChannelReader {
    fn fill_buf(&mut self) -> io::Result<&[u8]> {
        if self.pos >= self.buffer.len() && !self.finished {
            match self.rx.recv() {
                Ok(line) => {
                    self.buffer = line.into_bytes();
                    self.buffer.push(b'\n');
                    self.pos = 0;
                }
                Err(_) => self.finished = true,
            }
        }
        Ok(&self.buffer[self.pos..])
    }

    fn consume(&mut self, amt: usize) {
        self.pos += amt;
        if self.pos >= self.buffer.len() {
            self.buffer.clear();
            self.pos = 0;
        }
    }
}
