//! Implements hash to curve as described in Section 8.7.1 of
//! <https://datatracker.ietf.org/doc/draft-irtf-cfrg-hash-to-curve/?include_text=1>
//! and Section 5 of
//!  <https://eprint.iacr.org/2019/403.pdf>

use crate::error::HashingError;
use crate::isogeny::bls381g1::*;
use crate::{expand_message_xmd, expand_message_xof, DomainSeparationTag};
use crate::{HashToCurveXmd, HashToCurveXof};
use amcl_miracl::bls381::{big::BIG, dbig::DBIG, ecp::ECP};
use digest::{
    generic_array::typenum::{marker_traits::Unsigned, U128, U32, U64},
    BlockInput, Digest, ExtendableOutput, Input, Reset, XofReader,
};
use std::cmp::Ordering;

/// To compute a `L` use the following formula
/// L = ceil(ceil(log2(p) + k) / 8). For example, in our case log2(p) = 381, k = 128
/// L = 64
type L = U64;
type TwoL = U128;
const MODULUS: BIG = BIG {
    w: amcl_miracl::bls381::rom::MODULUS,
};
const PM1DIV2: BIG = BIG {
    w: [
        71916856549561685,
        108086211381297143,
        186063435852751093,
        218960087668936289,
        225643796693662629,
        229680090418738422,
        3490221905,
    ],
};
const H_EFF: BIG = BIG {
    w: [144396663052632065, 52, 0, 0, 0, 0, 0],
};
const C1: BIG = BIG {
    w: [
        132416828320029820,
        -36241730206030966,
        -183175740354038500,
        -108808289511770161,
        19716962043635886,
        150180602526288156,
        2033276157,
    ],
};
const C2: BIG = BIG {
    w: [
        170292360909944894,
        176868607242987704,
        7626954141253676,
        39810925030715689,
        14823383385055774,
        15557254971433191,
        634585801,
    ],
};
const SQRT_C1: BIG = BIG {
    w: [
        180073616350636715,
        198158293766504443,
        237146906002231418,
        253595231910324016,
        112821898346831314,
        258955233285225083,
        1745110952,
    ],
};

/// BLS12381G1_XMD:SHA-256_SSWU provides both
/// Random Oracle (RO)
/// Nonuniform (NU)
pub struct Bls12381G1Sswu {
    dst: DomainSeparationTag,
}

impl Bls12381G1Sswu {
    /// Create a new implementation with the default
    pub fn new(dst: DomainSeparationTag) -> Self { Self { dst } }
}

impl From<DomainSeparationTag> for Bls12381G1Sswu {
    fn from(dst: DomainSeparationTag) -> Self { Self { dst } }
}

impl HashToCurveXmd for Bls12381G1Sswu {
    type Output = ECP;

    fn encode_to_curve_xmd<D: BlockInput + Digest<OutputSize = U32>, I: AsRef<[u8]>>(
        &self,
        data: I,
    ) -> Result<Self::Output, HashingError> {
        let u = hash_to_field_xmd_nu::<D, I>(data, &self.dst)?;
        Ok(encode_to_curve(u))
    }

    fn hash_to_curve_xmd<D: BlockInput + Digest<OutputSize = U32>, I: AsRef<[u8]>>(
        &self,
        data: I,
    ) -> Result<Self::Output, HashingError> {
        let (u0, u1) = hash_to_field_xmd_ro::<D, I>(data, &self.dst)?;
        Ok(hash_to_curve(u0, u1))
    }
}

impl HashToCurveXof for Bls12381G1Sswu {
    type Output = ECP;

    fn encode_to_curve_xof<
        X: ExtendableOutput + Input + Reset + Default,
        D: Digest<OutputSize = U32>,
        I: AsRef<[u8]>,
    >(
        &self,
        data: I,
    ) -> Result<Self::Output, HashingError> {
        let u = hash_to_field_xof_nu::<X, D, I>(data, &self.dst)?;
        Ok(encode_to_curve(u))
    }

    fn hash_to_curve_xof<
        X: ExtendableOutput + Input + Reset + Default,
        D: Digest<OutputSize = U32>,
        I: AsRef<[u8]>,
    >(
        &self,
        data: I,
    ) -> Result<Self::Output, HashingError> {
        let (u0, u1) = hash_to_field_xof_ro::<X, D, I>(data, &self.dst)?;
        Ok(hash_to_curve(u0, u1))
    }
}

fn encode_to_curve(u: BIG) -> ECP {
    let q = map_to_curve(u);
    clear_cofactor(q)
}

fn hash_to_curve(u0: BIG, u1: BIG) -> ECP {
    let mut q0 = map_to_curve(u0);
    let q1 = map_to_curve(u1);
    q0.add(&q1);
    clear_cofactor(q0)
}

/// See Section 7 in
/// <https://datatracker.ietf.org/doc/draft-irtf-cfrg-hash-to-curve/?include_text=1>
fn clear_cofactor(p: ECP) -> ECP {
    p.mul(&H_EFF)
}

/// See Section 6.2 in
/// <https://datatracker.ietf.org/doc/draft-irtf-cfrg-hash-to-curve/?include_text=1>
fn map_to_curve(u: BIG) -> ECP {
    let (x, y) = map_to_curve_simple_swu(u);
    iso_map(x, y)
}

/// See Section 6.6.2.1 in
/// <https://datatracker.ietf.org/doc/draft-irtf-cfrg-hash-to-curve/?include_text=1>
///
/// Only works if p is congruent to 3 mod 4
fn map_to_curve_simple_swu(u: BIG) -> (BIG, BIG) {
    // tv1 = Z * u^2
    let tv1 = BIG::modmul(&Z, &BIG::modsqr(&u, &MODULUS), &MODULUS);
    // tv2 = tv1^2
    let mut tv2 = BIG::modsqr(&tv1, &MODULUS);

    // x1 = tv1 + tv2
    let mut x1 = BIG::new_big(&tv1);
    x1.add(&tv2);
    x1.rmod(&MODULUS);

    // x1 = inv0(x1)
    x1.invmodp(&MODULUS);

    let e1 = if x1.iszilch() { 1 } else { 0 };

    // x1 = x1 + 1
    x1.inc(1);

    // x1 = CMOV(x1, c2, e1)
    x1.cmove(&C2, e1);

    // x1 = x1 * c1
    x1 = BIG::modmul(&x1, &C1, &MODULUS);

    // gx1 = x1^2
    let mut gx1 = BIG::modsqr(&x1, &MODULUS);
    // gx1 = gx1 + A
    gx1.add(&ISO_A);
    gx1.rmod(&MODULUS);

    // gx1 = gx1 * x1
    gx1 = BIG::modmul(&gx1, &x1, &MODULUS);

    // gx1 = gx1 + B
    gx1.add(&ISO_B);
    gx1.rmod(&MODULUS);

    // x2 = tv1 * x1
    let x2 = BIG::modmul(&tv1, &x1, &MODULUS);

    // tv2 = tv1 * tv2
    tv2 = BIG::modmul(&tv1, &tv2, &MODULUS);

    // gx2 = gx1 * tv2
    let gx2 = BIG::modmul(&gx1, &tv2, &MODULUS);

    // e2 = is_square(gx1)
    let e2 = if is_square(&gx1) { 1 } else { 0 };

    // x = CMOV(x2, x1, e2)
    let mut x = BIG::new_copy(&x2);
    x.cmove(&x1, e2);

    // y2 = CMOV(gx2, gx1, e2)
    let mut y2 = BIG::new_copy(&gx2);
    y2.cmove(&gx1, e2);

    // y = sqrt(y2)
    let y = sqrt_3mod4(&y2);

    // e3 = sgn0(u) == sgn0(y)
    let e3 = if sgn0(&u) == sgn0(&y) { 1 } else { 0 };

    // y = CMOV(-y, y, e3)
    let mut y_neg = BIG::modneg(&y, &MODULUS);
    y_neg.cmove(&y, e3);

    (x, y_neg)
}

/// Section F.1 in
/// <https://datatracker.ietf.org/doc/draft-irtf-cfrg-hash-to-curve/?include_text=1>
fn sqrt_3mod4(x: &BIG) -> BIG {
    let mut t = BIG::new_big(x);
    t.powmod(&SQRT_C1, &MODULUS)
}

/// is_square(x) := { True,  if x^((q - 1) / 2) is 0 or 1 in F;
///                 { False, otherwise.
fn is_square(x: &BIG) -> bool {
    let mut t = BIG::new_copy(x);
    t = t.powmod(&PM1DIV2, &MODULUS);
    let mut sum = 0;
    for i in 1..t.w.len() {
        sum |= t.w[i];
    }
    sum == 0 && (t.w[0] == 0 || t.w[0] == 1)
}

/// See Section 4.1 in
/// <https://datatracker.ietf.org/doc/draft-irtf-cfrg-hash-to-curve/?include_text=1>
fn sgn0(x: &BIG) -> Ordering {
    if *x > PM1DIV2 {
        Ordering::Less
    } else {
        Ordering::Greater
    }
}

/// See Section 4.3 in
/// <https://eprint.iacr.org/2019/403.pdf>
fn iso_map(x_prime: BIG, y_prime: BIG) -> ECP {
    let mut x_values: [BIG; 16] = [BIG::new(); 16];
    x_values[0] = BIG::new_int(1);
    x_values[1] = x_prime;
    x_values[2] = BIG::modsqr(&x_prime, &MODULUS);
    x_values[3] = BIG::modmul(&x_values[2], &x_prime, &MODULUS);
    x_values[4] = BIG::modmul(&x_values[3], &x_prime, &MODULUS);
    x_values[5] = BIG::modmul(&x_values[4], &x_prime, &MODULUS);
    x_values[6] = BIG::modmul(&x_values[5], &x_prime, &MODULUS);
    x_values[7] = BIG::modmul(&x_values[6], &x_prime, &MODULUS);
    x_values[8] = BIG::modmul(&x_values[7], &x_prime, &MODULUS);
    x_values[9] = BIG::modmul(&x_values[8], &x_prime, &MODULUS);
    x_values[10] = BIG::modmul(&x_values[9], &x_prime, &MODULUS);
    x_values[11] = BIG::modmul(&x_values[10], &x_prime, &MODULUS);
    x_values[12] = BIG::modmul(&x_values[11], &x_prime, &MODULUS);
    x_values[13] = BIG::modmul(&x_values[12], &x_prime, &MODULUS);
    x_values[14] = BIG::modmul(&x_values[13], &x_prime, &MODULUS);
    x_values[15] = BIG::modmul(&x_values[14], &x_prime, &MODULUS);

    let mut x = iso_map_helper(&x_values, &X_NUM);
    let mut x_den = iso_map_helper(&x_values, &X_DEN);
    let mut y = iso_map_helper(&x_values, &Y_NUM);
    let mut y_den = iso_map_helper(&x_values, &Y_DEN);

    x_den.invmodp(&MODULUS);
    x = BIG::modmul(&x, &x_den, &MODULUS);

    y_den.invmodp(&MODULUS);
    y = BIG::modmul(&y, &y_den, &MODULUS);
    y = BIG::modmul(&y, &y_prime, &MODULUS);

    ECP::new_bigs(&x, &y)
}

/// Compute a section of iso map
fn iso_map_helper(x: &[BIG], k: &[BIG]) -> BIG {
    let mut new_x = BIG::new();
    for i in 0..k.len() {
        let t = BIG::modmul(&x[i], &k[i], &MODULUS);
        new_x.add(&t);
        new_x.rmod(&MODULUS);
    }
    new_x
}

/// Hash to field using expand_message_xmd to compute `u` as specified in Section 5.2 in
/// <https://datatracker.ietf.org/doc/draft-irtf-cfrg-hash-to-curve/?include_text=1>
fn hash_to_field_xmd_nu<D: BlockInput + Digest<OutputSize = U32>, M: AsRef<[u8]>>(
    msg: M,
    dst: &DomainSeparationTag,
) -> Result<BIG, HashingError> {
    // length_in_bytes = count * m * L = 1 * 1 * 64 = 64
    let random_bytes = expand_message_xmd::<M, D, L>(msg, dst)?;
    // elm_offset = L * (j + i * m) = 64 * (0 + 0 * 1) = 0
    // tv = substr(random_bytes, 0, 64)
    Ok(field_elem_from_larger_bytearray(random_bytes.as_slice()))
}

/// Hash to field using expand_message_xmd to compute two `u`s as specified in Section 5.2 in
/// <https://datatracker.ietf.org/doc/draft-irtf-cfrg-hash-to-curve/?include_text=1>
///
/// We avoid the loop and get compile time checking this way
fn hash_to_field_xmd_ro<D: BlockInput + Digest<OutputSize = U32>, M: AsRef<[u8]>>(
    msg: M,
    dst: &DomainSeparationTag,
) -> Result<(BIG, BIG), HashingError> {
    // length_in_bytes = count * m * L = 2 * 1 * 64 = 128
    let random_bytes = expand_message_xmd::<M, D, TwoL>(msg, dst)?;
    // elm_offset_0 = L * (j + i * m) = 64 * (0 + 0 * 1) = 0
    // elm_offset_1 = L * (j + i * m) = 64 * (0 + 1 * 1) = 64
    // tv_0 = substr(random_bytes, 0, 64)
    // tv_1 = substr(random_bytes, 64, 64)
    let u_0 = field_elem_from_larger_bytearray(&random_bytes[0..L::to_usize()]);
    let u_1 = field_elem_from_larger_bytearray(&random_bytes[L::to_usize()..]);
    Ok((u_0, u_1))
}

/// Hash to field using expand_message_xof to compute `u` as specified in Section 5.2 in
/// <https://datatracker.ietf.org/doc/draft-irtf-cfrg-hash-to-curve/?include_text=1>
fn hash_to_field_xof_nu<
    X: ExtendableOutput + Input + Reset + Default,
    D: Digest<OutputSize = U32>,
    M: AsRef<[u8]>,
>(
    msg: M,
    dst: &DomainSeparationTag,
) -> Result<BIG, HashingError> {
    // length_in_bytes = count * m * L = 1 * 1 * 64 = 64
    let random_bytes = expand_message_xof::<M, X, D, L>(msg, dst)?;
    // elm_offset = L * (j + i * m) = 64 * (0 + 0 * 1) = 0
    // tv = substr(random_bytes, 0, 64)
    Ok(field_elem_from_larger_bytearray(random_bytes.as_slice()))
}

/// Hash to field using expand_message_xof to compute two `u`s as specified in Section 5.2 in
/// <https://datatracker.ietf.org/doc/draft-irtf-cfrg-hash-to-curve/?include_text=1>
///
/// We avoid the loop and get compile time checking this way
fn hash_to_field_xof_ro<
    X: ExtendableOutput + Input + Reset + Default,
    D: Digest<OutputSize = U32>,
    M: AsRef<[u8]>,
>(
    msg: M,
    dst: &DomainSeparationTag,
) -> Result<(BIG, BIG), HashingError> {
    // length_in_bytes = count * m * L = 2 * 1 * 64 = 128
    let random_bytes = expand_message_xof::<M, X, D, TwoL>(msg, dst)?;
    // elm_offset_0 = L * (j + i * m) = 64 * (0 + 0 * 1) = 0
    // elm_offset_1 = L * (j + i * m) = 64 * (0 + 1 * 1) = 64
    // tv_0 = substr(random_bytes, 0, 64)
    // tv_1 = substr(random_bytes, 64, 64)
    let u_0 = field_elem_from_larger_bytearray(&random_bytes[0..L::to_usize()]);
    let u_1 = field_elem_from_larger_bytearray(&random_bytes[L::to_usize()..]);
    Ok((u_0, u_1))
}

/// FIELD_ELEMENT_SIZE <= random_bytes.len() <= FIELD_ELEMENT_SIZE * 2
fn field_elem_from_larger_bytearray(random_bytes: &[u8]) -> BIG {
    // e_j = OS2IP(tv) mod p
    let mut d = DBIG::new();
    for i in 0..random_bytes.len() {
        d.shl(8);
        d.w[0] += random_bytes[i] as amcl_miracl::arch::Chunk;
    }
    // u = (e_0, ..., e_( m - 1 ) )
    let u = d.dmod(&MODULUS);
    u
}

#[cfg(test)]
mod tests {
    use crate::bls381g1::{
        hash_to_field_xmd_nu, hash_to_field_xmd_ro, map_to_curve, Bls12381G1Sswu,
    };
    use crate::{DomainSeparationTag, HashToCurveXmd, HashToCurveXof};
    use amcl_miracl::bls381::{big::BIG, ecp::ECP};

    #[test]
    fn hash_to_curve_xmd_tests() {
        let dst = DomainSeparationTag::new(
            "BLS12381G1_XMD:SHA-256_SSWU_RO_",
            Some("TESTGEN"),
            None,
            None,
        )
        .unwrap();
        let msgs = [
            "",
            "abc",
            "abcdef0123456789",
            "a512_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
        ];
        let p = [
            ("14738daf70f5142df038c9e3be76f5d71b0db6613e5ef55cfe8e43e27f840dc75de97092da617376a9f598e7a0920c47", "12645b7cb071943631d062b22ca61a8a3df2a8bdac4e6fcd2c18643ef37a98beacf770ce28cb01c8abf5ed63d1a19b53"),
            ("01fea27a940188120178dfceec87dca78b745b6e73757be21c54d6cee6f07e3d5a465cf425c9d34dccfa95acffa86bf2", "18def9271f5fd253380c764a6818e8b6524c3d35864fcf963d85031225d62bf8cd0abeb326c3c62fec56f6100fa04367"),
            ("0bdbca067fc4458a1206ecf3e235b400449c5693dd99e99a9793da076cb65e1b796bc279c892ae1c320c3783e25062d2", "12ca3f12b93b0028390a4ef4fa7083cb23f66ca42423e6e53987620e1d57c23a0ad6a14db1f709d0494c7d5122e0632f"),
            ("0a81ca09b6a8c05712396801e6432a87b14ab1f764fa519e9f515816607283fe2a653a191fc1c8fee89cd30195e7a8e1", "11c7f1b59bb552692288da6557d1b5c72a448101faf56dd4125d8422af1425c4ddeecfbd5200525064657a79bdd0c3ed"),
        ];

        let blshasher = Bls12381G1Sswu::from(dst);

        for i in 0..msgs.len() {
            let expected_p = ECP::new_bigs(
                &BIG::from_hex(p[i].0.to_string()),
                &BIG::from_hex(p[i].1.to_string()),
            );
            let actual_p = blshasher.hash_to_curve_xmd::<sha2::Sha256, &str>(msgs[i]);
            assert!(actual_p.is_ok());
            let actual_p = actual_p.unwrap();
            assert_eq!(expected_p, actual_p);
        }
    }

    #[test]
    fn hash_to_curve_xof_tests() {
        let dst = DomainSeparationTag::new(
            "BLS12381G1_XOF:SHAKE-128_SSWU_RO_",
            Some("TESTGEN"),
            None,
            None,
        )
            .unwrap();
        let msgs = [
            "",
            "abc",
            "abcdef0123456789",
            "a512_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
        ];
        let p = [
            ("13BE5D0DA8916DC22A4683102E25158FCCF9695664B98E3CD41F723FEF99F92476FE5BFE495B787FDA5561AC6F0AB3B9", "10D5EFB4C0540BE37B23AB3A67324EA63D94AA5D7129210C7A4CA8AE5C48A3104CE74CEDC0117B56320CDD4242FEC1BC"),
            ("185782F1391BE17A64BC8ECB88CD7957118C0B968B7DBFAAEA4D0288BE243E4CF8CC10306AC58DED6994AC48837701BB", "12D37C727D8F5AF320B9DAB4E563D6E6578BBAAA1300EBEC58C1003C1A121669A53B39795F387AD510DA12E389B7CD6B"),
            ("19C3D7FF10EEF43623889C6221632C373A198AE108509A969D6B47A0D4ECA2483A884D2EAEA26A9214E6A54EBAD0E9C3", "025A8BD2768EC20CEA2E3D405FF72D4796BE83F8634317D1D70793591C6693954C91DEF9E6F553CE7ED4DC364CF05513"),
            ("0398FD7E656CEC001E1B3E1F88CA0CF6791A8F1C2C970E78E7E4E672EAD45340D53F958E20BF384FBB333F6F45328A1F", "0478D9837665E168D9AC3505C08AE122C504A78D8BE487012F078864D6C7043463E665F0DEA92EB6B374CADC65780A35"),
        ];

        let blshasher = Bls12381G1Sswu::from(dst);

        for i in 0..msgs.len() {
            let expected_p = ECP::new_bigs(
                &BIG::from_hex(p[i].0.to_string()),
                &BIG::from_hex(p[i].1.to_string()),
            );
            let actual_p = blshasher.hash_to_curve_xof::<sha3::Shake128, sha3::Sha3_256, &str>(msgs[i]);
            assert!(actual_p.is_ok());
            let actual_p = actual_p.unwrap();
            assert_eq!(expected_p, actual_p);
        }
    }

    #[test]
    fn encode_to_curve_xmd_tests() {
        let dst = DomainSeparationTag::new(
            "BLS12381G1_XMD:SHA-256_SSWU_NU_",
            Some("TESTGEN"),
            None,
            None,
        )
        .unwrap();
        let msgs = [
            "",
            "abc",
            "abcdef0123456789",
            "a512_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
        ];
        let p = [
            ("115281bd55a4103f31c8b12000d98149598b72e5da14e953277def263a24bc2e9fd8fa151df73ea3800f9c8cbb9b245c", "0796506faf9edbf1957ba8d667a079cab0d3a37e302e5132bd25665b66b26ea8556a0cfb92d6ae2c4890df0029b455ce"),
            ("04a7a63d24439ade3cd16eaab22583c95b061136bd5013cf109d92983f902c31f49c95cbeb97222577e571e97a68a32e", "09a8aa8d6e4b409bbe9a6976c016688269024d6e9d378ed25e8b4986194511f479228fa011ec88b8f4c57a621fc12187"),
            ("05c59faaf88187f51cd9cc6c20ca47ac66cc38d99af88aef2e82d7f35104168916f200a79562e64bc843f83cdc8a4675", "0b10472100a4aaa665f35f044b14a234b8f74990fa029e3dd06aa60b232fd9c232564ceead8cdb72a8a0320fc1071845"),
            ("10147709f8d4f6f2fa6f957f6c6533e3bf9069c01be721f9421d88e0f02d8c617d048c6f8b13b81309d1ef6b56eeddc7", "1048977c38688f1a3acf48ae319216cb1509b6a29bd1e7f3b2e476088a280e8c97d4a4c147f0203c7b3acb3caa566ae8"),
        ];

        let blshasher = Bls12381G1Sswu::from(dst);

        for i in 0..msgs.len() {
            let expected_p = ECP::new_bigs(
                &BIG::from_hex(p[i].0.to_string()),
                &BIG::from_hex(p[i].1.to_string()),
            );
            let actual_p = blshasher.encode_to_curve_xmd::<sha2::Sha256, &str>(msgs[i]);
            assert!(actual_p.is_ok());
            let actual_p = actual_p.unwrap();
            assert_eq!(expected_p, actual_p);
        }
    }

    #[test]
    fn map_to_curve_ro_tests() {
        let dst = DomainSeparationTag::new(
            "BLS12381G1_XMD:SHA-256_SSWU_RO_",
            Some("TESTGEN"),
            None,
            None,
        )
        .unwrap();
        let msgs = [
            "",
            "abc",
            "abcdef0123456789",
            "a512_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
        ];

        let expected_q = [
            ("02f2686965a4dd27ccb11119f2e131aefee818744a414d23ecef4db1407991fdf058f0affaee18fd586a9ab81060ae20",
             "0341a16c88a39b3d111b36b7cf885b7147b1d54b9201faaba5b47d7839bcf433cc35bb1f7b8e55aa9382a52fe4d84370",
             "1357bddd2bc6c8e752f3cf498ffe29ae87d8ff933701ae76f82d2839b0d9aee5229d4fff54dfb8223be0d88fa4485863",
             "09ba0ec3c78cf1e65330721f777b529aef27539642c39be11f459106b890ec5eb4a21c5d94885603e822cfa765170857"),

            ("119cc1d21e3e494d388a8718fe9f8ec6d8ff134486ce5c1f97129797616c4b8125f0dc568c59836cbf064496136438bc",
             "19e6c998825ee57b82c4808e4df477680f0f254c9edce228104422494a4e5d40d11ee676f6b861b6c49cf7de9d777aef",
             "0d1783f40bd83461b921c3fcd0e9ba326ef75272b122cf44338f0060d7179995a38ea9c66f3ce800e2f693d2634a4524",
             "017b2566d55fa7ee43844f1fa068cb0a11d5889c11607d939da046697c8ba25cf71054c2a8eb2189d3680485a39f5bdd"),

            ("1614d05720a39379fb89469883f90ae3e50995def9e17f8f8566a3f6cfb4fe88267eac1dc7834406fc597965065ef100",
             "1060e5aab331ac4940693a936ea80029bb2c4a3945add7ae35bce805e767af827c4a9ffcb5842fbc50ab234716d895f6",
             "0f612cda21cee750b1ccff361a4ce047e70d9a9e152e96a60aa29b5d8a5dcd25f7c5bd71bb56bd34e6a8af7532afaa4f",
             "1878f926302468949ef290b4fee621d1172e072eda1b42e366df68fc87f53c35583dbc043009e0b38a04a9b1ff617efe"),

            ("0a817078e7f30f08e94a25c2a1947160db1fe52042626660b8252cd339e678a1fecc0e6da60390a203532bd089a426b6",
             "097bd5d6ae3f5b5d0ba5e4099485caa2c505a1d900e4525af10254b3927ae0c82611be944ff8fdc6b278aab9e17ee27c",
             "1098f203da72c58dca61ffd52a3de82603d3154c527df51c2efe6298ea0eeaa065d57ba3a809b5e32d9d56dade119006",
             "0bcbd9df3505f049476f060c1d1c958fe8b34e426fd7e75424c9e227d9c4d3edbd5eddb8b1e89cc91b4a7bd3275d4d70"),
        ];

        for i in 0..msgs.len() {
            let u = hash_to_field_xmd_ro::<sha2::Sha256, &str>(msgs[i], &dst).unwrap();
            let exp_q = ECP::new_bigs(
                &BIG::from_hex(expected_q[i].0.to_string()),
                &BIG::from_hex(expected_q[i].1.to_string()),
            );
            let actual_q = map_to_curve(u.0);
            assert_eq!(exp_q, actual_q);
            let exp_q = ECP::new_bigs(
                &BIG::from_hex(expected_q[i].2.to_string()),
                &BIG::from_hex(expected_q[i].3.to_string()),
            );
            let actual_q = map_to_curve(u.1);
            assert_eq!(exp_q, actual_q);
        }
    }

    // Take from section G.9.2
    #[test]
    fn map_to_curve_nu_tests() {
        let dst = DomainSeparationTag::new(
            "BLS12381G1_XMD:SHA-256_SSWU_NU_",
            Some("TESTGEN"),
            None,
            None,
        )
        .unwrap();
        let msgs = [
            "",
            "abc",
            "abcdef0123456789",
            "a512_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
        ];
        let q_s = [
            ("0dddf77f320e7848a457358ab8d3b84cbaf19307be26b91a10c211651691cd736b1f59d77aed3954f857f108d6966f5b", "0450ab32020649f22a2fca166a1d8a59d4c93f1eb078a4bedd6c48027b9933507a2a8ae4d915305f58ede781283325a9"),
            ("12897a9a513b12303a7f0f3a3cc7c838d16847a31507980945312bede915848159bd390b16b8e378b398e31a385d9180", "1372530cc0811d70071e50640281aa8aaf96ee09c01281ccfead92296cb9dacf5054aa51dbea730e46239e709042a15d"),
            ("08459bd42a955d6e247fce6c81eda0ad9645f9e666d141a71f0afa3fbc509b2c58550fe077d073cc752493400399fddd", "169d35a8c6bb915ae910f4c6cde359622746b0c8b2b241b411d0e92ef991d3e6a7b0fafabb93c1de2e3997d6e362ce8a"),
            ("08c937d529c01ab2398b85b0bff6da465ed6265d4944dbbef7d383eea40157927082739c7b5417027d2225c6cb9d5ef0", "059047d83b5ea1ff7f0665b406acede27f233d3414055cbff25b37614b679f08fd6d807b5956edec6abad36c5321d99e"),
        ];

        for i in 0..msgs.len() {
            let u = hash_to_field_xmd_nu::<sha2::Sha256, &str>(msgs[i], &dst).unwrap();
            let expected_q = ECP::new_bigs(
                &BIG::from_hex(q_s[i].0.to_string()),
                &BIG::from_hex(q_s[i].1.to_string()),
            );
            let actual_q1 = map_to_curve(u);
            assert_eq!(expected_q, actual_q1);
        }
    }

    // Take from section G.9.2
    #[test]
    fn hash_to_field_xmd_nu_tests() {
        let dst = DomainSeparationTag::new(
            "BLS12381G1_XMD:SHA-256_SSWU_NU_",
            Some("TESTGEN"),
            None,
            None,
        )
        .unwrap();
        let msgs = [
            "",
            "abc",
            "abcdef0123456789",
            "a512_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
        ];
        let executed_u_s = [
            "0ccb6bda9b602ab82aae21c0291623e2f639648a6ada1c76d8ffb664130fd18d98a2cc6160624148827a9726678e7cd4",
            "08accd9a1bd4b75bb2e9f014ac354a198cbf607f0061d00a6286f5544cf4f9ecc1439e3194f570cbbc7b96d1a754f231",
            "0a359cf072db3a39acf22f086d825fcf49d0daf241d98902342380fc5130b44e55de8f684f300bc11c44dee526413363",
            "181d09392c52f7740d5eaae52123c1dfa4808343261d8bdbaf19e7773e5cdfd989165cd9ecc795500e5da2437dde2093",
        ];

        for i in 0..msgs.len() {
            let expected_u = BIG::from_hex(executed_u_s[i].to_string());
            let actual_u = hash_to_field_xmd_nu::<sha2::Sha256, &str>(msgs[i], &dst);
            assert!(actual_u.is_ok());
            assert_eq!(actual_u.unwrap(), expected_u);
        }
    }

    // Take from section G.9.1
    #[test]
    fn hash_to_field_xmd_ro_tests() {
        let dst = DomainSeparationTag::new(
            "BLS12381G1_XMD:SHA-256_SSWU_RO_",
            Some("TESTGEN"),
            None,
            None,
        )
        .unwrap();
        let msgs = [
            "",
            "abc",
            "abcdef0123456789",
            "a512_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
        ];
        let expected_u_s = [
            ("14700e34d15178550475044b044b4e41ca8d52a655c34f8afea856d21d499f48c9370d2bae4ae8351305493e48d36ab5", "17e2da57f6fd3f11dba6119db4cd26b03e63e67b4e42db678d9c41fdfcaff00ba336d8563abcd9da6c17d2e1784ee858"),
            ("10c84aa245c74ee20579a27e63199be5d19cdfb5e44c6b587765931605d7790a1df6e1433f78bcddb4edb8553374f75e", "0f73433dcc2b5f9905c49d905bd62e1a1529b057c77194e56d196860d9d645167e0430aec9d3c70de31dd046fcab4a20"),
            ("11503eb4a558d0d2c5fc7cdddb51ba715c33577cf1a7f2f21a7eee6d2a570332bbbe53ae3392c9f8d8f6c172ae484692", "0efd59b8d98be7c491dfdb9d2a669e32e9bb348f8a64dbf7e47708dd5d40f484b1439109a3f96230bf63af72b908c43d"),
            ("134dc7f817cc08c5a3128892385ff6e9dd55f5e39d9a2d74ac74058d5dfc025d507806ab5d9254bd2334defbb477400d", "0eeaf2c6f4c1ca5cc039d99cb94234f67e65968f36d9dd77e95da55dadd085b50fbb11489167ded9157e5aac0d99d5be"),
        ];

        for i in 0..msgs.len() {
            let expected_u0 = BIG::from_hex(expected_u_s[i].0.to_string());
            let expected_u1 = BIG::from_hex(expected_u_s[i].1.to_string());
            let res = hash_to_field_xmd_ro::<sha2::Sha256, &str>(msgs[i], &dst);
            assert!(res.is_ok());
            let (actual_u0, actual_u1) = res.unwrap();
            assert_eq!(actual_u0, expected_u0);
            assert_eq!(actual_u1, expected_u1);
        }
    }
}
