//! SipHash-2-4, implemented locally so that the on-disk hash format never depends
//! on a third-party crate's version or implementation details.

/// SipHash-2-4 streaming state.
///
/// The compression and finalization constants follow the reference
/// implementation: 2 compression rounds per message block and 4 finalization
/// rounds.
#[derive(Clone)]
pub(crate) struct SipHasher24 {
    v0: u64,
    v1: u64,
    v2: u64,
    v3: u64,
    /// Bytes of the partial block, little-endian packed.
    tail: u64,
    /// Number of valid bytes in `tail` (0..8).
    tail_len: u32,
    /// Total number of bytes written.
    length: u64,
}

impl SipHasher24 {
    pub(crate) const fn new(k0: u64, k1: u64) -> Self {
        Self {
            v0: k0 ^ 0x736f_6d65_7073_6575,
            v1: k1 ^ 0x646f_7261_6e64_6f6d,
            v2: k0 ^ 0x6c79_6765_6e65_7261,
            v3: k1 ^ 0x7465_6462_7974_6573,
            tail: 0,
            tail_len: 0,
            length: 0,
        }
    }

    /// Feeds bytes into the hasher. Chunk boundaries do not affect the result.
    pub(crate) fn write(&mut self, mut bytes: &[u8]) {
        self.length = self.length.wrapping_add(bytes.len() as u64);

        if self.tail_len != 0 {
            let needed = 8 - self.tail_len as usize;
            let take = needed.min(bytes.len());
            for (offset, &byte) in bytes[..take].iter().enumerate() {
                let shift = 8 * (self.tail_len as usize + offset);
                self.tail |= u64::from(byte) << shift;
            }
            self.tail_len += take as u32;
            bytes = &bytes[take..];
            if self.tail_len == 8 {
                self.compress_tail();
            }
        }

        while bytes.len() >= 8 {
            let block = u64::from_le_bytes(bytes[..8].try_into().expect("8-byte block"));
            self.v3 ^= block;
            self.compress_twice();
            self.v0 ^= block;
            bytes = &bytes[8..];
        }

        if !bytes.is_empty() {
            for (offset, &byte) in bytes.iter().enumerate() {
                self.tail |= u64::from(byte) << (8 * offset);
            }
            self.tail_len = bytes.len() as u32;
        }
    }

    pub(crate) fn finish(mut self) -> u64 {
        let last = self.tail | ((self.length & 0xff) << 56);
        self.v3 ^= last;
        self.compress_twice();
        self.v0 ^= last;
        self.v2 ^= 0xff;
        self.compress();
        self.compress();
        self.compress();
        self.compress();
        self.v0 ^ self.v1 ^ self.v2 ^ self.v3
    }

    fn compress_tail(&mut self) {
        self.v3 ^= self.tail;
        self.compress_twice();
        self.v0 ^= self.tail;
        self.tail = 0;
        self.tail_len = 0;
    }

    /// SipHash-2-4 applies two compression rounds per message block.
    #[inline]
    fn compress_twice(&mut self) {
        self.compress();
        self.compress();
    }

    #[inline]
    fn compress(&mut self) {
        self.v0 = self.v0.wrapping_add(self.v1);
        self.v1 = self.v1.rotate_left(13);
        self.v1 ^= self.v0;
        self.v0 = self.v0.rotate_left(32);

        self.v2 = self.v2.wrapping_add(self.v3);
        self.v3 = self.v3.rotate_left(16);
        self.v3 ^= self.v2;

        self.v0 = self.v0.wrapping_add(self.v3);
        self.v3 = self.v3.rotate_left(21);
        self.v3 ^= self.v0;

        self.v2 = self.v2.wrapping_add(self.v1);
        self.v1 = self.v1.rotate_left(17);
        self.v1 ^= self.v2;
        self.v2 = self.v2.rotate_left(32);
    }
}

#[cfg(test)]
mod tests {
    use super::SipHasher24;

    /// The 64-byte test vector prefix published with the SipHash reference
    /// implementation (`vectors.h`), for keys `00 01 .. 0f` and inputs
    /// `00`, `00 01`, ... `.` Each value is the little-endian u64 output.
    const VECTORS: [u64; 64] = [
        0x726f_db47_dd0e_0e31,
        0x74f8_39c5_93dc_67fd,
        0x0d6c_8009_d9a9_4f5a,
        0x8567_6696_d7fb_7e2d,
        0xcf27_94e0_2771_87b7,
        0x1876_5564_cd99_a68d,
        0xcbc9_466e_58fe_e3ce,
        0xab02_00f5_8b01_d137,
        0x93f5_f579_9a93_2462,
        0x9e00_82df_0ba9_e4b0,
        0x7a5d_bbc5_94dd_b9f3,
        0xf4b3_2f46_226b_ada7,
        0x751e_8fbc_860e_e5fb,
        0x14ea_5627_c084_3d90,
        0xf723_ca90_8e7a_f2ee,
        0xa129_ca61_49be_45e5,
        0x3f2a_cc7f_57c2_9bdb,
        0x699a_e9f5_2cbe_4794,
        0x4bc1_b3f0_968d_d39c,
        0xbb6d_c91d_a779_61bd,
        0xbed6_5cf2_1aa2_ee98,
        0xd0f2_cbb0_2e3b_67c7,
        0x9353_6795_e3a3_3e88,
        0xa80c_038c_cd5c_cec8,
        0xb8ad_50c6_f649_af94,
        0xbce1_92de_8a85_b8ea,
        0x17d8_35b8_5bbb_15f3,
        0x2f2e_6163_076b_cfad,
        0xde4d_aaac_a71d_c9a5,
        0xa6a2_5066_8795_6571,
        0xad87_a353_5c49_ef28,
        0x32d8_92fa_d841_c342,
        0x7127_512f_72f2_7cce,
        0xa7f3_2346_f959_78e3,
        0x12e0_b01a_bb05_1238,
        0x15e0_34d4_0fa1_97ae,
        0x314d_ffbe_0815_a3b4,
        0x0279_90f0_2962_3981,
        0xcadc_d4e5_9ef4_0c4d,
        0x9abf_d876_6a33_735c,
        0x0e3e_a96b_5304_a7d0,
        0xad0c_42d6_fc58_5992,
        0x1873_06c8_9bc2_15a9,
        0xd4a6_0abc_f379_2b95,
        0xf935_451d_e4f2_1df2,
        0xa953_8f04_1975_5787,
        0xdb9a_cddf_f56c_a510,
        0xd06c_98cd_5c09_75eb,
        0xe612_a3cb_9ecb_a951,
        0xc766_e62c_fcad_af96,
        0xee64_435a_9752_fe72,
        0xa192_d576_b245_165a,
        0x0a87_87bf_8ecb_74b2,
        0x81b3_e73d_20b4_9b6f,
        0x7fa8_220b_a3b2_ecea,
        0x2457_31c1_3ca4_2499,
        0xb78d_bfaf_3a8d_83bd,
        0xea1a_d565_322a_1a0b,
        0x60e6_1c23_a379_5013,
        0x6606_d7e4_4628_2b93,
        0x6ca4_ecb1_5c5f_91e1,
        0x9f62_6da1_5c96_25f3,
        0xe51b_3860_8ef2_5f57,
        0x958a_324c_eb06_4572,
    ];

    fn reference_hash(bytes: &[u8]) -> u64 {
        let mut key = [0u8; 16];
        for (index, slot) in key.iter_mut().enumerate() {
            *slot = u8::try_from(index).expect("index fits in u8");
        }
        let k0 = u64::from_le_bytes(key[..8].try_into().expect("8-byte key half"));
        let k1 = u64::from_le_bytes(key[8..].try_into().expect("8-byte key half"));
        let mut hasher = SipHasher24::new(k0, k1);
        hasher.write(bytes);
        hasher.finish()
    }

    #[test]
    fn matches_reference_vectors() {
        let mut message = Vec::new();
        for (length, expected) in VECTORS.iter().enumerate() {
            assert_eq!(
                reference_hash(&message),
                *expected,
                "mismatch for {length}-byte message"
            );
            message.push(u8::try_from(length).expect("index fits in u8"));
        }
    }

    #[test]
    fn chunking_does_not_change_the_hash() {
        let message: Vec<u8> = (0..=255u8).collect();
        let mut whole = SipHasher24::new(1, 2);
        whole.write(&message);
        let whole = whole.finish();

        for chunk_size in [1, 3, 7, 8, 9, 16, 63] {
            let mut split = SipHasher24::new(1, 2);
            for chunk in message.chunks(chunk_size) {
                split.write(chunk);
            }
            assert_eq!(split.finish(), whole, "chunk size {chunk_size}");
        }
    }

    #[test]
    fn keys_change_the_output() {
        let mut a = SipHasher24::new(0, 0);
        a.write(b"seal");
        let mut b = SipHasher24::new(1, 0);
        b.write(b"seal");
        assert_ne!(a.finish(), b.finish());
    }
}
