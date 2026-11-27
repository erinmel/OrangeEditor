//! The in-place radix-2 fast Fourier transform under the spectrum snapshots, public so
//! an application analysing its own audio does not need a second copy.
//!
//! A transform has a power-of-two size of at least 2. Its input goes in bit-reversed
//! order ([`reversed`]); [`transform`] then leaves bin `k` at index `k`.
use std::f64::consts::TAU;

/// The rotation factors `(cos, sin)` of a transform of `size` points.
#[expect(clippy::cast_precision_loss, reason = "transform sizes are small")]
pub fn twiddles(size: usize) -> Vec<(f64, f64)> {
    (0..size / 2)
        .map(|index| {
            let (sin, cos) = (-TAU * index as f64 / size as f64).sin_cos();
            (cos, sin)
        })
        .collect()
}

/// Where input `index` of a `size`-point transform goes: its bits reversed.
pub fn reversed(index: usize, size: usize) -> usize {
    index.reverse_bits() >> (usize::BITS - size.ilog2())
}

/// Transforms `data`, filled in bit-reversed order, in place with the factors of
/// [`twiddles`] for its length.
pub fn transform(data: &mut [(f64, f64)], twiddles: &[(f64, f64)]) {
    let size = data.len();
    let mut span = 2;
    while span <= size {
        let stride = size / span;
        for chunk in data.chunks_exact_mut(span) {
            let (left, right) = chunk.split_at_mut(span / 2);
            for (index, (a, b)) in left.iter_mut().zip(right).enumerate() {
                let (c, s) = twiddles[index * stride];
                let (real, imag) = (b.0 * c - b.1 * s, b.0 * s + b.1 * c);
                *b = (a.0 - real, a.1 - imag);
                *a = (a.0 + real, a.1 + imag);
            }
        }
        span *= 2;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A cosine with a whole number of cycles lands in one bin, at half its peak times
    /// the size on each side of the spectrum.
    #[test]
    fn a_cosine_lands_in_its_bin() {
        const SIZE: usize = 64;
        let mut data = vec![(0.0, 0.0); SIZE];
        for index in 0..SIZE {
            #[expect(clippy::cast_precision_loss, reason = "a small test size")]
            let value = (TAU * 5.0 * index as f64 / SIZE as f64).cos();
            data[reversed(index, SIZE)] = (value, 0.0);
        }
        transform(&mut data, &twiddles(SIZE));
        let magnitude = |(real, imag): (f64, f64)| real.hypot(imag);
        assert!((magnitude(data[5]) - 32.0).abs() < 1e-9);
        assert!((magnitude(data[SIZE - 5]) - 32.0).abs() < 1e-9);
        for (bin, value) in data.iter().enumerate() {
            if bin != 5 && bin != SIZE - 5 {
                assert!(magnitude(*value) < 1e-9, "bin {bin}");
            }
        }
    }
}
