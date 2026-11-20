//! Packets on the pipe between the app and the core: `[kind: u8][len: u32 LE][payload]`.
//! `J` carries one JSON value; `F` carries a frame: display, width, height and stride (u32 LE each)
//! followed by the BGRA bytes.

use std::io::{self, Read, Write};

/// Bigger than any frame we accept (8K BGRA with alignment), so a corrupt length cannot make the
/// reader allocate without bound.
pub const MAX_PACKET: usize = 8192 * 4352 * 4 + 16;

#[derive(Debug, PartialEq)]
pub enum Packet {
    Json(serde_json::Value),
    Frame(Frame),
}

#[derive(Debug, PartialEq)]
pub struct Frame {
    pub display: u32,
    pub width: u32,
    pub height: u32,
    pub stride: u32,
    pub bgra: Vec<u8>,
}

pub fn write_json<W: Write>(w: &mut W, value: &impl serde::Serialize) -> io::Result<()> {
    let body = serde_json::to_vec(value).map_err(io::Error::other)?;
    w.write_all(&[b'J'])?;
    w.write_all(&(body.len() as u32).to_le_bytes())?;
    w.write_all(&body)?;
    w.flush()
}

pub fn write_frame<W: Write>(w: &mut W, display: u32, width: u32, height: u32, stride: u32, bgra: &[u8]) -> io::Result<()> {
    if (stride as usize) < width as usize * 4 || bgra.len() < stride as usize * height as usize {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "frame smaller than its stride × height"));
    }
    let body = &bgra[..stride as usize * height as usize];
    w.write_all(&[b'F'])?;
    w.write_all(&((body.len() + 16) as u32).to_le_bytes())?;
    for v in [display, width, height, stride] {
        w.write_all(&v.to_le_bytes())?;
    }
    w.write_all(body)?;
    w.flush()
}

fn u32_at(b: &[u8], i: usize) -> u32 {
    u32::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]])
}

/// Reads one packet; `Ok(None)` at a clean end of stream.
pub fn read_packet<R: Read>(r: &mut R) -> io::Result<Option<Packet>> {
    let mut head = [0u8; 5];
    match r.read_exact(&mut head[..1]) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e),
    }
    r.read_exact(&mut head[1..])?;
    let len = u32_at(&head, 1) as usize;
    if len > MAX_PACKET {
        return Err(io::Error::new(io::ErrorKind::InvalidData, format!("packet of {len} bytes")));
    }
    let mut body = vec![0u8; len];
    r.read_exact(&mut body)?;
    match head[0] {
        b'J' => Ok(Some(Packet::Json(serde_json::from_slice(&body).map_err(io::Error::other)?))),
        b'F' => {
            if len < 16 {
                return Err(io::Error::new(io::ErrorKind::InvalidData, "short frame"));
            }
            let (display, width, height, stride) = (u32_at(&body, 0), u32_at(&body, 4), u32_at(&body, 8), u32_at(&body, 12));
            if (stride as usize) < width as usize * 4 || len - 16 != stride as usize * height as usize {
                return Err(io::Error::new(io::ErrorKind::InvalidData, "frame size does not match its header"));
            }
            body.drain(..16);
            Ok(Some(Packet::Frame(Frame { display, width, height, stride, bgra: body })))
        }
        k => Err(io::Error::new(io::ErrorKind::InvalidData, format!("unknown packet kind {k}"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_and_frame_round_trip() {
        let mut buf = Vec::new();
        write_json(&mut buf, &serde_json::json!({"event": "closed"})).unwrap();
        let px: Vec<u8> = (0..(3 * 8 * 2)).map(|i| i as u8).collect();
        write_frame(&mut buf, 1, 2, 3, 8, &px).unwrap();
        let mut r = &buf[..];
        assert_eq!(read_packet(&mut r).unwrap(), Some(Packet::Json(serde_json::json!({"event": "closed"}))));
        match read_packet(&mut r).unwrap() {
            Some(Packet::Frame(f)) => {
                assert_eq!((f.display, f.width, f.height, f.stride), (1, 2, 3, 8));
                assert_eq!(f.bgra.len(), 24);
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(read_packet(&mut r).unwrap(), None);
    }

    #[test]
    fn rejects_lies_about_size() {
        // header says 2×3 with stride 8 (24 bytes) but carries 4
        let mut buf = vec![b'F'];
        buf.extend_from_slice(&20u32.to_le_bytes());
        for v in [0u32, 2, 3, 8] {
            buf.extend_from_slice(&v.to_le_bytes());
        }
        buf.extend_from_slice(&[0; 4]);
        assert!(read_packet(&mut &buf[..]).is_err());
        // a length beyond the cap is refused before allocating
        let mut big = vec![b'J'];
        big.extend_from_slice(&u32::MAX.to_le_bytes());
        assert!(read_packet(&mut &big[..]).is_err());
        // stride smaller than a row
        assert!(write_frame(&mut Vec::new(), 0, 4, 1, 8, &[0; 8]).is_err());
    }
}
