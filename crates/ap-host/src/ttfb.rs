//! Time-to-first-byte recording output pipe (leaf 03.05): wraps a
//! [`MemoryOutputPipe`] so the host can measure hello-world → first output
//! byte WITHOUT changing what the module observes (still a plain WASI
//! stdout pipe; the measurement is host-side).
//!
//! Intercept path: wasi p1 fd_write goes through the `p2_stream()`
//! adapter (the `wasmtime_wasi::p2::OutputStream` trait object), so the
//! stamp fires in the `OutputStream::write` override.
//!
//! SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception

use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use wasmtime_wasi::cli::{IsTerminal, StdoutStream};
use wasmtime_wasi::p2::pipe::MemoryOutputPipe;
use wasmtime_wasi_io::poll::Pollable;
use wasmtime_wasi_io::streams::{OutputStream, StreamError};

/// The async-trait-desugared signature OutputStream::cancel carries.
type CancelFuture<'a> = Pin<Box<dyn Future<Output = ()> + Send + 'a>>;

/// Shared measurement cell (the pipe is cloned per stream acquisition).
#[derive(Debug, Default, Clone)]
pub struct FirstByteStamp(Arc<Mutex<Option<Instant>>>);

impl FirstByteStamp {
    /// Elapsed from `start` to the first write, in microseconds (None =
    /// the module never wrote to this fd).
    pub fn elapsed_us_since(&self, start: Instant) -> Option<u128> {
        // The stamp instant is taken AFTER `start` (the run began first),
        // so elapsed = t.saturating_duration_since(start).
        self.0
            .lock()
            .ok()?
            .map(|t| t.saturating_duration_since(start).as_micros())
    }
}

/// A [`StdoutStream`] that records WHEN the first byte lands.
#[derive(Clone)]
pub struct FirstBytePipe {
    inner: MemoryOutputPipe,
    stamp: FirstByteStamp,
}

impl FirstBytePipe {
    /// Wrap an EXISTING capture pipe (the host drain keeps reading the
    /// original MemoryOutputPipe — the buffer is Arc-shared, so wrapping
    /// adds the stamp without forking the captured bytes).
    pub fn wrap(inner: MemoryOutputPipe, stamp: FirstByteStamp) -> Self {
        Self { inner, stamp }
    }

    /// Bound the captured pipe at `cap` bytes; `stamp` receives the first
    /// write's Instant.
    pub fn new(cap: usize, stamp: FirstByteStamp) -> Self {
        Self::wrap(MemoryOutputPipe::new(cap), stamp)
    }

    /// Drain the captured bytes (same semantics as MemoryOutputPipe).
    pub fn contents(&self) -> Vec<u8> {
        self.inner.contents().to_vec()
    }

    fn fire(&self, n: usize) {
        if n > 0 {
            if let Ok(mut cell) = self.stamp.0.lock() {
                if cell.is_none() {
                    *cell = Some(Instant::now());
                }
            }
        }
    }
}

impl OutputStream for FirstBytePipe {
    fn write(&mut self, bytes: bytes::Bytes) -> Result<(), StreamError> {
        self.fire(bytes.len());
        self.inner.write(bytes)
    }

    fn flush(&mut self) -> Result<(), StreamError> {
        self.inner.flush()
    }

    fn check_write(&mut self) -> Result<usize, StreamError> {
        self.inner.check_write()
    }

    fn cancel<'life0, 'async_trait>(
        &'life0 mut self,
    ) -> Pin<Box<dyn Future<Output = ()> + Send + 'async_trait>>
    where
        'life0: 'async_trait,
        Self: 'async_trait,
    {
        self.inner.cancel()
    }
}

#[async_trait::async_trait]
impl Pollable for FirstBytePipe {
    async fn ready(&mut self) {
        self.inner.ready().await;
    }
}

impl StdoutStream for FirstBytePipe {
    fn async_stream(&self) -> Box<dyn tokio::io::AsyncWrite + Send + Sync> {
        self.inner.async_stream()
    }

    // The intercept lives on THIS path (p1 fd_write): hand out a clone of
    // self whose OutputStream::write fires the stamp before delegating.
    fn p2_stream(&self) -> Box<dyn wasmtime_wasi_io::streams::OutputStream> {
        Box::new(self.clone())
    }
}

impl IsTerminal for FirstBytePipe {
    // A capture pipe is never a terminal (matches MemoryOutputPipe).
    fn is_terminal(&self) -> bool {
        false
    }
}
