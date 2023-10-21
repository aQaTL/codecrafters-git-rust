/// Software SHA1 implementation
/// Source 1: https://en.wikipedia.org/wiki/SHA-1
/// Source 2: https://csrc.nist.gov/files/pubs/fips/180-2/upd1/final/docs/fips180-2withchangenotice.pdf
pub fn sha1(data: &[u8]) -> [u8; 20] {
	let mut h: [u32; 5] = [0x67452301, 0xEFCDAB89, 0x98BADCFE, 0x10325476, 0xC3D2E1F0];

	let mut data = data.to_vec();

	let message_len_in_bits = data.len() * 8;
	data.push(0x80_u8);

	let mut padding_needed = 448_i64 - ((message_len_in_bits + 1).rem_euclid(512) as i64);
	if padding_needed < 0 {
		padding_needed = (512 - ((message_len_in_bits + 1).rem_euclid(512) as i64)) + 448;
	}

	padding_needed -= 7;
	debug_assert_eq!(padding_needed % 8, 0);

	let byte_padding_needed = padding_needed / 8;
	data.extend(std::iter::repeat(0_u8).take(byte_padding_needed as usize));
	data.extend(message_len_in_bits.to_be_bytes());

	let data_u32: &mut [u32] =
		unsafe { std::slice::from_raw_parts_mut(data.as_mut_ptr().cast::<u32>(), data.len() / 4) };

	// Convert the message into 32bit big endian words.
	data_u32.iter_mut().for_each(|n| *n = n.to_be());

	for chunk in data_u32.chunks_exact_mut(16) {
		let mut w = [0_u32; 80];
		w[0..16].copy_from_slice(chunk);

		// Message schedule: extend the sixteen 32-bit words into eighty 32-bit words:
		for i in 16..=79 {
			w[i] = (w[i - 3] ^ w[i - 8] ^ w[i - 14] ^ w[i - 16]).rotate_left(1);
		}

		// Initialize hash value for this chunk:
		let mut a = h[0];
		let mut b = h[1];
		let mut c = h[2];
		let mut d = h[3];
		let mut e = h[4];

		// Main loop
		for (idx, word) in w.into_iter().enumerate() {
			let f: u32;
			let k: u32;

			match idx {
				0..=19 => {
					f = (b & c) | ((!b) & d);
					k = 0x5A827999;
				}
				20..=39 => {
					f = b ^ c ^ d;
					k = 0x6ED9EBA1;
				}
				40..=59 => {
					f = (b & c) | (b & d) | (c & d);
					k = 0x8F1BBCDC;
				}
				60..=79 => {
					f = b ^ c ^ d;
					k = 0xCA62C1D6;
				}
				_ => unreachable!("w idx range not covered"),
			}

			let temp: u32 = (a.rotate_left(5))
				.wrapping_add(f)
				.wrapping_add(e)
				.wrapping_add(k)
				.wrapping_add(word);
			e = d;
			d = c;
			c = b.rotate_left(30);
			b = a;
			a = temp;
		}

		h[0] = h[0].wrapping_add(a);
		h[1] = h[1].wrapping_add(b);
		h[2] = h[2].wrapping_add(c);
		h[3] = h[3].wrapping_add(d);
		h[4] = h[4].wrapping_add(e);
	}

	let mut out = [0u8; 20];
	for i in 0..5 {
		let slice = &mut out[(i * 4)..((i + 1) * 4)];
		slice.copy_from_slice(h[i].to_be_bytes().as_slice());
	}
	out
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn empty() {
		let result = sha1(&[]);
		assert_eq!(
			result,
			[
				0xda, 0x39, 0xa3, 0xee, 0x5e, 0x6b, 0x4b, 0x0d, 0x32, 0x55, 0xbf, 0xef, 0x95, 0x60,
				0x18, 0x90, 0xaf, 0xd8, 0x07, 0x09
			]
		);
	}

	#[test]
	fn simple() {
		let data = "The quick brown fox jumps over the lazy dog";
		let result = sha1(data.as_bytes());
		assert_eq!(
			result,
			[
				0x2f, 0xd4, 0xe1, 0xc6, 0x7a, 0x2d, 0x28, 0xfc, 0xed, 0x84, 0x9e, 0xe1, 0xbb, 0x76,
				0xe7, 0x39, 0x1b, 0x93, 0xeb, 0x12
			]
		);

		let hex_str = hex::encode(result);
		assert_eq!(hex_str, "2fd4e1c67a2d28fced849ee1bb76e7391b93eb12");
	}
}
