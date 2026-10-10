//! Eventos SSE del anillo de logs (`GET /api/events`).
//!
//! Extraído de `server.rs` (era `mod sse` interno): `EventReader` sirve primero
//! el history con `seq` y luego el live por `mpsc`, con padding a 1100 B para
//! forzar el flush del `BufWriter` de 1 KB de `tiny_http`. Sin estado global,
//! sin red: el llamador (`server.rs`) lo envuelve en la respuesta SSE.

use crate::process::LogEvent;
use std::io::Read;
use std::sync::mpsc;

/// SSE EventReader: primero history, luego live por mpsc.
pub struct EventReader {
    rx: std::sync::Mutex<mpsc::Receiver<String>>,
    initial: Vec<LogEvent>,
    next_live_seq: u64,
    done: bool,
    chunk: Vec<u8>,
}

impl EventReader {
    pub fn new(rx: mpsc::Receiver<String>, initial: Vec<LogEvent>) -> Self {
        let next_live_seq = initial.last().map(|l| l.seq + 1).unwrap_or(0);
        Self {
            rx: std::sync::Mutex::new(rx),
            initial,
            next_live_seq,
            done: false,
            chunk: Vec::new(),
        }
    }

    fn encode(&self, seq: u64, line: &str) -> Vec<u8> {
        let data = serde_json::json!({ "type":"log","seq": seq, "line": line });
        // Pad to force tiny_http BufWriter (1KB) flush — SSE events must hit the client live.
        let mut s = format!("data: {}\n: pad\n\n", data);
        let min = 1100;
        if s.len() < min {
            let pad = min - s.len() - 3;
            s.push_str(": ");
            s.push_str(&" ".repeat(pad));
            s.push('\n');
        }
        s.into_bytes()
    }
}

impl Read for EventReader {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if self.chunk.is_empty() {
            if self.done {
                return Ok(0);
            }
            if let Some(ev) = if self.initial.is_empty() {
                None
            } else {
                Some(self.initial.remove(0))
            } {
                self.chunk = self.encode(ev.seq, &ev.line);
            } else {
                let g = match self.rx.lock() {
                    Ok(g) => g,
                    Err(po) => po.into_inner(),
                };
                // P1 producción: `recv_timeout` en vez de `recv()` bloqueante.
                // Un cliente que se va sin leer ya no fuga el hilo de
                // respuesta para siempre; cada 25 s sale un heartbeat `: ping`
                // (forza flush y detecta desconexión en la siguiente escritura).
                match g.recv_timeout(std::time::Duration::from_secs(25)) {
                    Ok(line) => {
                        let seq = self.next_live_seq;
                        self.next_live_seq += 1;
                        self.chunk = self.encode(seq, &line);
                    }
                    Err(mpsc::RecvTimeoutError::Timeout) => {
                        self.chunk = b": ping\n\n".to_vec();
                    }
                    Err(_) => {
                        self.done = true;
                        return Ok(0);
                    }
                }
            }
        }
        let n = self.chunk.len().min(buf.len());
        buf[..n].copy_from_slice(&self.chunk[..n]);
        Ok(n)
    }
}
