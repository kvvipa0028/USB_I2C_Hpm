//! Counter-continuity verification for the device-side u32 pattern stream.

/// One place where the received counter jumped unexpectedly.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Discontinuity {
    /// Absolute word index (across all bytes checked so far) where the jump
    /// was detected.
    pub word_offset: u64,
    pub expected: u32,
    pub got: u32,
}

/// Verifies that a stream of little-endian u32 words is an unbroken
/// incrementing sequence, continuous across calls (transfer boundaries) and
/// across u32 wraparound.
pub struct Verifier {
    expected: u32,
    started: bool,
    word_index: u64,
    /// Total discontinuities seen (not capped by `keep`).
    pub count: u64,
    /// Details of the first `keep` discontinuities.
    pub samples: Vec<Discontinuity>,
    keep: usize,
}

impl Default for Verifier {
    fn default() -> Self {
        Self::new()
    }
}

impl Verifier {
    pub fn new() -> Self {
        Self {
            expected: 0,
            started: false,
            word_index: 0,
            count: 0,
            samples: Vec::new(),
            keep: 16,
        }
    }

    /// Check a slice of pattern words. `words` must already be decoded to
    /// u32 (host byte order).
    pub fn check(&mut self, words: &[u32]) {
        for &w in words {
            if self.started && w != self.expected {
                self.count += 1;
                if self.samples.len() < self.keep {
                    self.samples.push(Discontinuity {
                        word_offset: self.word_index,
                        expected: self.expected,
                        got: w,
                    });
                }
            }
            self.expected = w.wrapping_add(1);
            self.started = true;
            self.word_index += 1;
        }
    }

    /// Convenience: check a byte slice of little-endian u32s. Trailing
    /// incomplete bytes are ignored (transfer granularity is always a
    /// multiple of 4 from the device).
    pub fn check_bytes(&mut self, bytes: &[u8]) {
        for chunk in bytes.chunks_exact(4) {
            let w = u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
            self.check(&[w]);
        }
    }

    /// Total words checked so far.
    pub fn words(&self) -> u64 {
        self.word_index
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn continuous_stream() {
        let mut v = Verifier::new();
        let words: Vec<u32> = (0..1000).collect();
        v.check(&words);
        assert_eq!(v.count, 0);
        assert_eq!(v.words(), 1000);
    }

    #[test]
    fn dropped_word() {
        let mut v = Verifier::new();
        let mut words: Vec<u32> = (0..100).collect();
        words.remove(40); // drop word 40
        v.check(&words);
        assert_eq!(v.count, 1);
        assert_eq!(
            v.samples[0],
            Discontinuity {
                word_offset: 40,
                expected: 40,
                got: 41,
            }
        );
    }

    #[test]
    fn duplicated_word() {
        let mut v = Verifier::new();
        let mut words: Vec<u32> = (0..100).collect();
        words.insert(55, words[54]); // repeat word 54
        v.check(&words);
        assert_eq!(v.count, 1);
        assert_eq!(
            v.samples[0],
            Discontinuity {
                word_offset: 55,
                expected: 55,
                got: 54,
            }
        );
        // resync: the rest of the stream is still continuous
        assert_eq!(v.samples.len(), 1);
    }

    #[test]
    fn chunk_boundary_split() {
        let mut v = Verifier::new();
        let words: Vec<u32> = (0..500).collect();
        v.check(&words[..173]);
        v.check(&words[173..]);
        assert_eq!(v.count, 0);
        assert_eq!(v.words(), 500);
    }

    #[test]
    fn wraparound() {
        let mut v = Verifier::new();
        let words: Vec<u32> = vec![u32::MAX - 1, u32::MAX, 0, 1, 2];
        v.check(&words);
        assert_eq!(v.count, 0);
    }

    #[test]
    fn resync_after_gap() {
        let mut v = Verifier::new();
        let words: Vec<u32> = vec![0, 1, 2, 100, 101, 102];
        v.check(&words);
        assert_eq!(v.count, 1);
        assert_eq!(v.samples[0].expected, 3);
        assert_eq!(v.samples[0].got, 100);
    }

    #[test]
    fn check_bytes_le() {
        let mut v = Verifier::new();
        let words: Vec<u32> = (0..10).collect();
        let bytes: Vec<u8> = words.iter().flat_map(|w| w.to_le_bytes()).collect();
        v.check_bytes(&bytes);
        assert_eq!(v.count, 0);
        assert_eq!(v.words(), 10);
    }
}
