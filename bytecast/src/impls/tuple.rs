use crate::{BytesError, FromBytes, ToBytes};

const fn add_max_size(a: Option<usize>, b: Option<usize>) -> Option<usize> {
    match (a, b) {
        (Some(a), Some(b)) => a.checked_add(b),
        _ => None,
    }
}

macro_rules! impl_tuple {
    ($($idx:tt $T:ident),+) => {
        impl<$($T: ToBytes),+> ToBytes for ($($T,)+) {
            const MAX_SIZE: Option<usize> = {
                let size = Some(0usize);
                $(
                    let size = add_max_size(size, <$T as ToBytes>::MAX_SIZE);
                )+
                size
            };

            fn to_bytes(&self, buf: &mut [u8]) -> Result<usize, BytesError> {
                let mut offset = 0;
                $(
                    offset += self.$idx.to_bytes(&mut buf[offset..])?;
                )+
                Ok(offset)
            }

            fn byte_len(&self) -> Option<usize> {
                let mut total = 0;
                $(
                    total += self.$idx.byte_len()?;
                )+
                Some(total)
            }
        }

        #[allow(non_snake_case)]
        impl<$($T: FromBytes),+> FromBytes for ($($T,)+) {
            fn from_bytes(buf: &[u8]) -> Result<(Self, usize), BytesError> {
                let mut offset = 0;
                $(
                    let ($T, n) = <$T as FromBytes>::from_bytes(&buf[offset..])?;
                    offset += n;
                )+
                Ok((($($T,)+), offset))
            }
        }
    };
}

impl_tuple!(0 A);
impl_tuple!(0 A, 1 B);
impl_tuple!(0 A, 1 B, 2 C);
impl_tuple!(0 A, 1 B, 2 C, 3 D);
impl_tuple!(0 A, 1 B, 2 C, 3 D, 4 E);
impl_tuple!(0 A, 1 B, 2 C, 3 D, 4 E, 5 F);
impl_tuple!(0 A, 1 B, 2 C, 3 D, 4 E, 5 F, 6 G);
impl_tuple!(0 A, 1 B, 2 C, 3 D, 4 E, 5 F, 6 G, 7 H);
impl_tuple!(0 A, 1 B, 2 C, 3 D, 4 E, 5 F, 6 G, 7 H, 8 I);
impl_tuple!(0 A, 1 B, 2 C, 3 D, 4 E, 5 F, 6 G, 7 H, 8 I, 9 J);
impl_tuple!(0 A, 1 B, 2 C, 3 D, 4 E, 5 F, 6 G, 7 H, 8 I, 9 J, 10 K);
impl_tuple!(0 A, 1 B, 2 C, 3 D, 4 E, 5 F, 6 G, 7 H, 8 I, 9 J, 10 K, 11 L);
