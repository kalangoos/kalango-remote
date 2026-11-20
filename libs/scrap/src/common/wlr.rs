//! Kalango Remote: capture through wlr-screencopy (niri and other compositors without the
//! RemoteDesktop portal). Used when `wlr_remote::active()`: Kalango Remote's core turns it on after
//! checking the compositor offers the protocols, and runs RustDesk's X11-shaped code paths with it.

use crate::{common::TraitCapturer, Frame, Pixfmt};
use std::{io, time::Duration};

use super::x11::PixelBuffer;

pub struct Display {
    index: usize,
    output: wlr_remote::Output,
}

impl Display {
    pub fn all() -> io::Result<Vec<Display>> {
        Ok(wlr_remote::outputs()?
            .into_iter()
            .enumerate()
            .map(|(index, output)| Display { index, output })
            .collect())
    }

    pub fn primary() -> io::Result<Display> {
        Self::all()?
            .into_iter()
            .next()
            .ok_or_else(|| io::ErrorKind::NotFound.into())
    }

    pub fn width(&self) -> usize {
        self.output.width as usize
    }

    pub fn height(&self) -> usize {
        self.output.height as usize
    }

    pub fn logical_width(&self) -> usize {
        self.output.logical_width as usize
    }

    pub fn logical_height(&self) -> usize {
        self.output.logical_height as usize
    }

    pub fn scale(&self) -> f64 {
        if self.output.logical_width > 0 {
            self.output.width as f64 / self.output.logical_width as f64
        } else {
            1.0
        }
    }

    pub fn origin(&self) -> (i32, i32) {
        (self.output.x, self.output.y)
    }

    pub fn is_online(&self) -> bool {
        true
    }

    pub fn is_primary(&self) -> bool {
        self.index == 0
    }

    pub fn name(&self) -> String {
        self.output.name.clone()
    }
}

pub struct Capturer {
    capture: wlr_remote::Capture,
    seq: u64,
    frame: Vec<u8>,
    width: usize,
    height: usize,
}

impl Capturer {
    pub fn new(display: Display) -> io::Result<Capturer> {
        Ok(Capturer {
            capture: wlr_remote::Capture::start(display.index)?,
            seq: 0,
            frame: Vec::new(),
            width: display.width(),
            height: display.height(),
        })
    }

    pub fn width(&self) -> usize {
        self.width
    }

    pub fn height(&self) -> usize {
        self.height
    }
}

impl TraitCapturer for Capturer {
    fn frame<'a>(&'a mut self, timeout: Duration) -> io::Result<Frame<'a>> {
        match self.capture.next(self.seq, timeout)? {
            Some(f) => {
                self.seq = f.seq;
                self.width = f.width;
                self.height = f.height;
                self.frame = f.bgra;
            }
            // nothing changed on screen: the video service treats this as "no new frame"
            None => return Err(io::ErrorKind::WouldBlock.into()),
        }
        Ok(Frame::PixelBuffer(PixelBuffer::new(
            &self.frame,
            Pixfmt::BGRA,
            self.width,
            self.height,
        )))
    }
}
