//! `rpcs3-rsx-fifo` — RSX/GCM command-buffer (FIFO) decoder.
//!
//! Ports the command-word decoding of `rpcs3/Emu/RSX/FIFO.cpp`.
//! The RSX command ring is a buffer of big-endian u32 words. The
//! command processor walks it from the GET pointer toward PUT,
//! decoding each header word into either a run of method writes or a
//! control-flow command (jump / call / return / nop).
//!
//! This crate is pure and allocation-light: [`decode`] reads one
//! header (plus its argument words) at a byte offset and returns a
//! [`Decoded`] describing the entry and where GET advances to. It
//! performs **no** rendering and holds no GPU state — it is the
//! bottom, behavior-freezable layer that the RSX state machine
//! (R12.3+) consumes.
//!
//! ## Command word format (NV / RSX)
//!
//! Given header `cmd` (host-endian, already converted from the
//! big-endian ring):
//!
//! | Pattern (`cmd & mask == val`) | Meaning |
//! |---|---|
//! | `& 0xe0000003 == 0x20000000` | OLD JUMP → GET = `cmd & 0x1ffffffc` |
//! | `& 0x00000003 == 0x00000001` | NEW JUMP → GET = `cmd & 0xfffffffc` |
//! | `& 0x00000003 == 0x00000002` | CALL → push GET+4, GET = `cmd & 0xfffffffc` |
//! | `cmd == 0x00020000` | RETURN → GET = call-stack pop |
//! | otherwise | method: count=`(cmd>>18)&0x7ff`, reg=`(cmd&0x3fffc)>>2`, non-increment=`cmd&0x40000000` |
//!
//! A method header with `count == 0` carries no args (treated as a
//! NOP-like no-op write run).
//!
//! ## Register indexing
//!
//! The wire method field is a byte offset (multiple of 4); this crate
//! exposes it as a **register index** = `byte_offset / 4`, matching
//! the `methods[reg]` register-file model the RSX state layer uses.
//! Increment methods advance the register index by 1 per argument.

// =====================================================================
// Command-word masks (NV FIFO)
// =====================================================================

const OLD_JUMP_MASK: u32 = 0xe000_0003;
const OLD_JUMP_CMD: u32 = 0x2000_0000;
const OLD_JUMP_OFFSET_MASK: u32 = 0x1fff_fffc;

const LOW2_MASK: u32 = 0x0000_0003;
const NEW_JUMP_CMD: u32 = 0x0000_0001;
const CALL_CMD: u32 = 0x0000_0002;
const JUMP_CALL_OFFSET_MASK: u32 = 0xffff_fffc;

const RETURN_CMD: u32 = 0x0002_0000;

const NON_INCREMENT_FLAG: u32 = 0x4000_0000;
const COUNT_SHIFT: u32 = 18;
const COUNT_MASK: u32 = 0x7ff;
const METHOD_OFFSET_MASK: u32 = 0x0003_fffc;

// =====================================================================
// Decoded entry
// =====================================================================

/// One decoded FIFO entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FifoEntry {
    /// A run of method-register writes as `(register_index, arg)`
    /// pairs. For increment methods the register advances by 1 per
    /// arg; for non-increment all writes target the same register.
    /// Empty when the header's count is 0.
    Methods(Vec<(u32, u32)>),
    /// Unconditional jump — GET becomes the byte offset.
    Jump(u32),
    /// Subroutine call — push the return offset, GET becomes the
    /// byte offset.
    Call(u32),
    /// Return — GET is restored from the call stack.
    Return,
    /// No-op header.
    Nop,
}

/// Result of decoding one header (plus its args) at a byte offset.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Decoded {
    /// The decoded entry.
    pub entry: FifoEntry,
    /// Byte offset of the next sequential header (after this header
    /// and any argument words). For `Jump`/`Call`/`Return` the engine
    /// overrides GET with the target instead of using this; it is
    /// still provided for completeness (= the fall-through offset).
    pub next_get: u32,
}

/// Errors from decoding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FifoError {
    /// `get` (or an argument word) lies outside the command buffer.
    OutOfBounds,
    /// `get` is not 4-byte aligned.
    Misaligned,
    /// A RETURN was decoded with an empty call stack.
    EmptyCallStack,
    /// The engine processed more than [`MAX_FIFO_ITERS`] entries
    /// without reaching PUT (suspected jump loop).
    RunawayFifo,
}

// =====================================================================
// Decoder
// =====================================================================

#[inline]
fn read_u32_be(buf: &[u8], byte_off: u32) -> Result<u32, FifoError> {
    let i = byte_off as usize;
    let end = i.checked_add(4).ok_or(FifoError::OutOfBounds)?;
    if end > buf.len() {
        return Err(FifoError::OutOfBounds);
    }
    Ok(u32::from_be_bytes([buf[i], buf[i + 1], buf[i + 2], buf[i + 3]]))
}

/// Decode the FIFO header at byte offset `get` in the command ring
/// `buf` (raw big-endian bytes).
pub fn decode(buf: &[u8], get: u32) -> Result<Decoded, FifoError> {
    if get & 0x3 != 0 {
        return Err(FifoError::Misaligned);
    }
    let cmd = read_u32_be(buf, get)?;

    // Control-flow forms first.
    if cmd & OLD_JUMP_MASK == OLD_JUMP_CMD {
        return Ok(Decoded {
            entry: FifoEntry::Jump(cmd & OLD_JUMP_OFFSET_MASK),
            next_get: get.wrapping_add(4),
        });
    }
    if cmd == RETURN_CMD {
        return Ok(Decoded {
            entry: FifoEntry::Return,
            next_get: get.wrapping_add(4),
        });
    }
    match cmd & LOW2_MASK {
        NEW_JUMP_CMD => {
            return Ok(Decoded {
                entry: FifoEntry::Jump(cmd & JUMP_CALL_OFFSET_MASK),
                next_get: get.wrapping_add(4),
            });
        }
        CALL_CMD => {
            return Ok(Decoded {
                entry: FifoEntry::Call(cmd & JUMP_CALL_OFFSET_MASK),
                next_get: get.wrapping_add(4),
            });
        }
        _ => {}
    }

    // Method header.
    let count = (cmd >> COUNT_SHIFT) & COUNT_MASK;
    let reg = (cmd & METHOD_OFFSET_MASK) >> 2;
    let non_increment = cmd & NON_INCREMENT_FLAG != 0;

    if count == 0 {
        return Ok(Decoded {
            entry: FifoEntry::Nop,
            next_get: get.wrapping_add(4),
        });
    }

    let mut writes = Vec::with_capacity(count as usize);
    for i in 0..count {
        let arg = read_u32_be(buf, get.wrapping_add(4).wrapping_add(i * 4))?;
        let target = if non_increment { reg } else { reg + i };
        writes.push((target, arg));
    }
    let next_get = get.wrapping_add(4).wrapping_add(count * 4);
    Ok(Decoded { entry: FifoEntry::Methods(writes), next_get })
}

// =====================================================================
// R12.2 — FIFO engine (DMA control: PUT/GET + call stack)
// =====================================================================

/// Maximum FIFO entries processed in one [`FifoEngine::run`] before
/// bailing with [`FifoError::RunawayFifo`]. Guards against malformed
/// jump loops in untrusted command streams.
pub const MAX_FIFO_ITERS: usize = 1_000_000;

/// The RSX command-processor DMA control state: a GET pointer that
/// chases the PUT pointer through the command ring, plus a
/// subroutine call stack for CALL/RETURN.
#[derive(Debug, Clone, Default)]
pub struct FifoEngine {
    /// Current read offset (byte offset into the command buffer).
    pub get: u32,
    /// Write offset the GET pointer chases toward.
    pub put: u32,
    /// CALL/RETURN return-offset stack.
    call_stack: Vec<u32>,
}

impl FifoEngine {
    /// Create an engine positioned at `get`, chasing `put`.
    #[must_use]
    pub fn new(get: u32, put: u32) -> Self {
        Self { get, put, call_stack: Vec::new() }
    }

    /// Current call-stack depth (test/diagnostic helper).
    #[must_use]
    pub fn call_depth(&self) -> usize {
        self.call_stack.len()
    }

    /// Process commands from GET until it reaches PUT, following
    /// jump/call/return, and return every `(register, arg)` method
    /// write in order. Stops cleanly when `get == put`.
    pub fn run(&mut self, buf: &[u8]) -> Result<Vec<(u32, u32)>, FifoError> {
        let mut writes = Vec::new();
        let mut iters = 0usize;
        while self.get != self.put {
            iters += 1;
            if iters > MAX_FIFO_ITERS {
                return Err(FifoError::RunawayFifo);
            }
            let d = decode(buf, self.get)?;
            match d.entry {
                FifoEntry::Methods(mut w) => {
                    writes.append(&mut w);
                    self.get = d.next_get;
                }
                FifoEntry::Nop => {
                    self.get = d.next_get;
                }
                FifoEntry::Jump(addr) => {
                    self.get = addr;
                }
                FifoEntry::Call(addr) => {
                    // Return to the header after this CALL.
                    self.call_stack.push(d.next_get);
                    self.get = addr;
                }
                FifoEntry::Return => {
                    self.get = self.call_stack.pop().ok_or(FifoError::EmptyCallStack)?;
                }
            }
        }
        Ok(writes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a big-endian byte buffer from u32 words.
    fn words(ws: &[u32]) -> Vec<u8> {
        let mut v = Vec::with_capacity(ws.len() * 4);
        for w in ws {
            v.extend_from_slice(&w.to_be_bytes());
        }
        v
    }

    #[test]
    fn increment_method_advances_register() {
        // count=3, reg byte offset 0x100 (reg index 0x40), increment.
        let header = (3 << 18) | 0x100;
        let buf = words(&[header, 0xAA, 0xBB, 0xCC]);
        let d = decode(&buf, 0).unwrap();
        assert_eq!(
            d.entry,
            FifoEntry::Methods(vec![(0x40, 0xAA), (0x41, 0xBB), (0x42, 0xCC)])
        );
        assert_eq!(d.next_get, 16);
    }

    #[test]
    fn non_increment_method_repeats_register() {
        let header = NON_INCREMENT_FLAG | (3 << 18) | 0x100;
        let buf = words(&[header, 0x11, 0x22, 0x33]);
        let d = decode(&buf, 0).unwrap();
        assert_eq!(
            d.entry,
            FifoEntry::Methods(vec![(0x40, 0x11), (0x40, 0x22), (0x40, 0x33)])
        );
    }

    #[test]
    fn old_jump_decodes_offset() {
        let cmd = OLD_JUMP_CMD | 0x1000;
        let buf = words(&[cmd]);
        let d = decode(&buf, 0).unwrap();
        assert_eq!(d.entry, FifoEntry::Jump(0x1000));
    }

    #[test]
    fn new_jump_decodes_offset() {
        let cmd = 0x2000 | NEW_JUMP_CMD; // low2 == 1
        let buf = words(&[cmd]);
        let d = decode(&buf, 0).unwrap();
        assert_eq!(d.entry, FifoEntry::Jump(0x2000));
    }

    #[test]
    fn call_decodes_offset() {
        let cmd = 0x3000 | CALL_CMD; // low2 == 2
        let buf = words(&[cmd]);
        let d = decode(&buf, 0).unwrap();
        assert_eq!(d.entry, FifoEntry::Call(0x3000));
    }

    #[test]
    fn return_decodes() {
        let buf = words(&[RETURN_CMD]);
        let d = decode(&buf, 0).unwrap();
        assert_eq!(d.entry, FifoEntry::Return);
    }

    #[test]
    fn zero_count_header_is_nop() {
        let buf = words(&[0x0000_0000]); // count 0, reg 0
        let d = decode(&buf, 0).unwrap();
        assert_eq!(d.entry, FifoEntry::Nop);
        assert_eq!(d.next_get, 4);
    }

    #[test]
    fn misaligned_get_rejected() {
        let buf = words(&[0, 0]);
        assert_eq!(decode(&buf, 2), Err(FifoError::Misaligned));
    }

    #[test]
    fn out_of_bounds_header_rejected() {
        let buf = words(&[0]);
        assert_eq!(decode(&buf, 8), Err(FifoError::OutOfBounds));
    }

    #[test]
    fn out_of_bounds_arg_rejected() {
        // header claims 2 args but buffer only holds the header + 1.
        let header = (2 << 18) | 0x100;
        let buf = words(&[header, 0xAA]);
        assert_eq!(decode(&buf, 0), Err(FifoError::OutOfBounds));
    }

    #[test]
    fn sequential_walk_via_next_get() {
        // two back-to-back single-arg increment methods.
        let h0 = (1 << 18) | 0x100;
        let h1 = (1 << 18) | 0x200;
        let buf = words(&[h0, 0xDE, h1, 0xAD]);
        let d0 = decode(&buf, 0).unwrap();
        assert_eq!(d0.entry, FifoEntry::Methods(vec![(0x40, 0xDE)]));
        assert_eq!(d0.next_get, 8);
        let d1 = decode(&buf, d0.next_get).unwrap();
        assert_eq!(d1.entry, FifoEntry::Methods(vec![(0x80, 0xAD)]));
    }

    // ---- R12.2: FifoEngine ----------------------------------------

    #[test]
    fn engine_runs_linear_until_put() {
        let h0 = (1 << 18) | 0x100;
        let h1 = (2 << 18) | 0x200;
        let buf = words(&[h0, 0xDE, h1, 0x11, 0x22]);
        let mut eng = FifoEngine::new(0, 20); // put at end (5 words)
        let writes = eng.run(&buf).unwrap();
        assert_eq!(
            writes,
            vec![(0x40, 0xDE), (0x80, 0x11), (0x81, 0x22)]
        );
        assert_eq!(eng.get, eng.put);
    }

    #[test]
    fn engine_follows_jump() {
        // word0: jump to byte 8; word2: method.
        let jump = OLD_JUMP_CMD | 8;
        let h = (1 << 18) | 0x100;
        let buf = words(&[jump, 0xDEAD, h, 0xBE]);
        let mut eng = FifoEngine::new(0, 16);
        let writes = eng.run(&buf).unwrap();
        // word1 (0xDEAD) is skipped by the jump.
        assert_eq!(writes, vec![(0x40, 0xBE)]);
    }

    #[test]
    fn engine_call_and_return() {
        // Layout (bytes) — subroutine sits BEFORE the main stream so
        // the CALL target never coincides with PUT:
        //   0: m_sub reg 0x80 = 0xBB   (subroutine body)
        //   8: return
        //  12: call → 0                (GET starts here)
        //  16: m_main reg 0x40 = 0xAA  (after return)
        //  PUT = 24
        let m_sub = (1 << 18) | 0x200;  // reg 0x80
        let m_main = (1 << 18) | 0x100; // reg 0x40
        let call = 0u32 | CALL_CMD;     // target 0, low2 == 2
        let buf = words(&[
            m_sub, 0xBB,    // 0, 4
            RETURN_CMD,     // 8
            call,           // 12
            m_main, 0xAA,   // 16, 20
        ]);
        let mut eng = FifoEngine::new(12, 24);
        let writes = eng.run(&buf).unwrap();
        // call→sub: (0x80,0xBB); return→16: (0x40,0xAA); get=24=PUT.
        assert_eq!(writes, vec![(0x80, 0xBB), (0x40, 0xAA)]);
        assert_eq!(eng.call_depth(), 0);
    }

    #[test]
    fn engine_return_without_call_errors() {
        let buf = words(&[RETURN_CMD, 0]);
        let mut eng = FifoEngine::new(0, 8);
        assert_eq!(eng.run(&buf), Err(FifoError::EmptyCallStack));
    }

    #[test]
    fn engine_runaway_jump_loop_bails() {
        // jump-to-self at byte 0.
        let buf = words(&[OLD_JUMP_CMD | 0]);
        let mut eng = FifoEngine::new(0, 4);
        assert_eq!(eng.run(&buf), Err(FifoError::RunawayFifo));
    }

    #[test]
    fn engine_empty_when_get_equals_put() {
        let buf = words(&[(1 << 18) | 0x100, 0xAA]);
        let mut eng = FifoEngine::new(8, 8);
        assert_eq!(eng.run(&buf).unwrap(), vec![]);
    }
}
