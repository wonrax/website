//! A command's output cut down to what the model reads

use std::collections::VecDeque;

/// Bytes kept from the start of the output
const HEAD_BYTES: usize = 1024;
/// Bytes kept from the end, where the errors and results usually are
const TAIL_BYTES: usize = 4096;

/// Collects a command's output, keeping only its start and end once it outgrows both. Tool
/// results ride along in the session for the rest of it.
#[derive(Default)]
pub struct Capture {
    head: Vec<u8>,
    tail: VecDeque<u8>,
    total: usize,
}

impl Capture {
    pub fn push(&mut self, bytes: &[u8]) {
        self.total += bytes.len();
        let room = HEAD_BYTES.saturating_sub(self.head.len()).min(bytes.len());
        let (head, rest) = bytes.split_at(room);
        self.head.extend_from_slice(head);
        self.tail.extend(rest);
        let excess = self.tail.len().saturating_sub(TAIL_BYTES);
        self.tail.drain(..excess);
    }

    /// The text kept, and how many bytes were cut out of its middle
    pub fn finish(self) -> (String, usize) {
        let omitted = self
            .total
            .saturating_sub(self.head.len() + self.tail.len());
        let mut tail = Vec::from(self.tail);
        if omitted == 0 {
            let mut all = self.head;
            all.append(&mut tail);
            return (String::from_utf8_lossy(&all).into_owned(), 0);
        }

        // The cuts can split a character: drop its stray bytes rather than show them garbled
        let head = String::from_utf8_lossy(&self.head);
        let head = head.trim_end_matches(char::REPLACEMENT_CHARACTER);
        let continuation = tail
            .iter()
            .take(3)
            .take_while(|byte| **byte & 0xC0 == 0x80)
            .count();
        let tail = String::from_utf8_lossy(&tail[continuation..]);
        (
            format!("{head}\n[... {omitted} bytes cut ...]\n{tail}"),
            omitted,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_output_is_kept_whole() {
        let mut capture = Capture::default();
        capture.push(b"hello ");
        capture.push(b"world");
        assert_eq!(capture.finish(), ("hello world".to_string(), 0));

        let mut capture = Capture::default();
        capture.push(&vec![b'x'; HEAD_BYTES + TAIL_BYTES]);
        assert_eq!(capture.finish().1, 0);
    }

    #[test]
    fn long_output_keeps_its_start_and_end() {
        let mut capture = Capture::default();
        capture.push(&vec![b'a'; HEAD_BYTES]);
        for _ in 0..100 {
            capture.push(&[b'm'; 100]);
        }
        capture.push(&vec![b'z'; TAIL_BYTES]);
        let (text, omitted) = capture.finish();
        assert_eq!(omitted, 100 * 100);
        assert!(text.starts_with(&"a".repeat(HEAD_BYTES)));
        assert!(text.ends_with(&"z".repeat(TAIL_BYTES)));
        assert!(!text.contains('m'));
    }

    #[test]
    fn cuts_through_a_character_leave_no_garbage() {
        let mut capture = Capture::default();
        capture.push(&vec![b'a'; HEAD_BYTES - 1]);
        // Two-byte characters, the first split across the end of the head
        capture.push("é".repeat(5000).as_bytes());
        let (text, _) = capture.finish();
        assert!(!text.contains(char::REPLACEMENT_CHARACTER), "{text}");
    }
}
