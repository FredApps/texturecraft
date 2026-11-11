//! The USD Crate binary format (`.usdc`, versions 0.4.0 and newer): bootstrap header, table of
//! contents, the TOKENS / STRINGS / FIELDS / FIELDSETS / PATHS / SPECS sections (LZ4 blocks and
//! the integer coding of [`codec`]), and the values a mesh needs: numeric scalars, vectors,
//! matrices and arrays (inlined, raw or compressed), tokens, strings, asset paths, token and
//! path vectors, token / path / reference list ops and the first time sample.

use std::collections::HashMap;

use super::codec::{self, Scalar, ty};
use super::{Budget, KIND, Layer, Prop, Specifier, Val};
use crate::{FormatError, Result, malformed};

/// Oldest Crate version read (compressed structural sections).
const MIN_VERSION: (u8, u8, u8) = (0, 4, 0);
const MAX_SECTIONS: usize = 64;
/// Items in one table or array.
const MAX_ITEMS: usize = 1 << 28;

/// Spec types.
const SPEC_ATTRIBUTE: i64 = 1;
const SPEC_PRIM: i64 = 6;
const SPEC_PSEUDO_ROOT: i64 = 7;
const SPEC_RELATIONSHIP: i64 = 8;
const SPEC_VARIANT: i64 = 10;
const SPEC_VARIANT_SET: i64 = 11;

fn truncated() -> FormatError {
    malformed(KIND, "the file is truncated or an offset points outside it")
}

/// A bounds-checked little-endian cursor.
#[derive(Clone)]
struct Cur<'a> {
    b: &'a [u8],
    pos: usize,
}

impl<'a> Cur<'a> {
    fn bytes(&mut self, n: usize) -> Result<&'a [u8]> {
        let s = self.b.get(self.pos..self.pos.checked_add(n).ok_or_else(truncated)?).ok_or_else(truncated)?;
        self.pos += n;
        Ok(s)
    }

    fn array<const N: usize>(&mut self) -> Result<[u8; N]> {
        self.bytes(N)?.try_into().map_err(|_| truncated())
    }

    fn u8(&mut self) -> Result<u8> {
        let [b] = self.array::<1>()?;
        Ok(b)
    }

    fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_le_bytes(self.array()?))
    }

    fn u64(&mut self) -> Result<u64> {
        Ok(u64::from_le_bytes(self.array()?))
    }

    fn f64(&mut self) -> Result<f64> {
        Ok(f64::from_le_bytes(self.array()?))
    }

    fn remaining(&self) -> usize {
        self.b.len().saturating_sub(self.pos)
    }

    /// A count of items of `item_bytes` each that must fit in the rest of the data (0: items
    /// that follow compressed, bounded by [`MAX_ITEMS`] only).
    fn count(&mut self, item_bytes: usize) -> Result<usize> {
        let n = self.u64()?;
        codec::check_count(n, self.cap(item_bytes))
    }

    /// Most items of `item_bytes` each the rest of the data can hold (0: [`MAX_ITEMS`]).
    fn cap(&self, item_bytes: usize) -> usize {
        self.remaining().checked_div(item_bytes).map_or(MAX_ITEMS, |n| n.min(MAX_ITEMS))
    }

    fn seek(&mut self, pos: u64) -> Result<()> {
        self.pos = usize::try_from(pos).ok().filter(|p| *p <= self.b.len()).ok_or_else(truncated)?;
        Ok(())
    }

    /// Follow a relative int64 offset stored at the cursor (Crate's nested values).
    fn jump(&mut self) -> Result<()> {
        let start = self.pos as u64;
        let off = i64::from_le_bytes(self.array()?);
        self.seek(start.checked_add_signed(off).filter(|_| off >= 8).ok_or_else(truncated)?)
    }

    /// `n` integers of `width` bytes in the compressed integer coding.
    fn ints(&mut self, n: usize, width: usize, budget: &mut Budget) -> Result<Vec<i64>> {
        let size = self.u64()?;
        let comp = self.bytes(usize::try_from(size).map_err(|_| truncated())?)?;
        if n == 0 {
            return Ok(Vec::new());
        }
        // LZ4 expands at most ~255×; a coded integer takes at least 2 bits.
        if n > comp.len().saturating_mul(1024).saturating_add(64) {
            return Err(malformed(KIND, "integer array is larger than its compressed data allows"));
        }
        budget.take(n.saturating_mul(8))?;
        let raw = codec::decompress(comp, codec::encoded_ints_len(n, width))?;
        codec::decode_ints(&raw, n, width)
    }
}

/// A little-endian scalar from its `s.size()` bytes.
fn scalar(s: Scalar, b: &[u8]) -> f64 {
    let mut a = [0u8; 8];
    for (o, v) in a.iter_mut().zip(b.iter().take(s.size())) {
        *o = *v;
    }
    let le = u64::from_le_bytes(a);
    match s {
        Scalar::U8 => f64::from(le as u8),
        Scalar::I32 => f64::from(le as u32 as i32),
        Scalar::U32 => f64::from(le as u32),
        Scalar::I64 => le as i64 as f64,
        Scalar::U64 => le as f64,
        Scalar::F16 => f64::from(half::f16::from_bits(le as u16).to_f32()),
        Scalar::F32 => f64::from(f32::from_bits(le as u32)),
        Scalar::F64 => f64::from_bits(le),
    }
}

fn is_quat(t: u8) -> bool {
    (ty::QUATD..=ty::QUATH).contains(&t)
}

/// Quaternions are stored imaginary first; values keep USD's text order (real first).
fn quat_order(t: u8, v: &mut [f64]) {
    if is_quat(t) {
        for q in v.as_chunks_mut::<4>().0 {
            q.rotate_right(1);
        }
    }
}

#[derive(Clone)]
struct PathEl {
    text: String,
    parent: Option<usize>,
    name: String,
    prop: bool,
}

struct Crate<'a> {
    b: &'a [u8],
    ver: (u8, u8, u8),
    tokens: Vec<String>,
    strings: Vec<u32>,
    fields: Vec<(u32, u64)>,
    fieldsets: Vec<u32>,
    paths: Vec<Option<PathEl>>,
}

pub(super) fn read(bytes: &[u8], budget: &mut Budget) -> Result<Layer> {
    let mut boot = Cur { b: bytes, pos: 8 };
    let [major, minor, patch] = boot.array::<3>()?;
    let ver = (major, minor, patch);
    if ver < MIN_VERSION {
        return Err(FormatError::Unsupported(format!("USD Crate version {major}.{minor}.{patch} (TextureCraft reads 0.4.0 and newer)")));
    }
    boot.seek(16)?;
    let mut toc = Cur { b: bytes, pos: 0 };
    toc.seek(boot.u64()?)?;
    let n = toc.count(32)?.min(MAX_SECTIONS);
    let mut sections = HashMap::new();
    for _ in 0..n {
        let name = toc.array::<16>()?;
        let name = String::from_utf8_lossy(name.split(|&c| c == 0).next().unwrap_or_default()).into_owned();
        let (start, size) = (toc.u64()?, toc.u64()?);
        let end = start.checked_add(size).filter(|e| *e <= bytes.len() as u64).ok_or_else(truncated)?;
        sections.insert(name, bytes.get(start as usize..end as usize).ok_or_else(truncated)?);
    }
    let section = |name: &str| sections.get(name).map(|b| Cur { b, pos: 0 }).ok_or_else(|| malformed(KIND, format!("missing {name} section")));
    let mut c = Crate { b: bytes, ver, tokens: Vec::new(), strings: Vec::new(), fields: Vec::new(), fieldsets: Vec::new(), paths: Vec::new() };
    c.read_tokens(section("TOKENS")?, budget)?;
    if let Ok(mut s) = section("STRINGS") {
        let n = s.count(4)?;
        c.strings = (0..n).map(|_| s.u32()).collect::<Result<_>>()?;
    }
    c.read_fields(section("FIELDS")?, budget)?;
    let mut s = section("FIELDSETS")?;
    let n = s.count(0)?;
    c.fieldsets = s.ints(n, 4, budget)?.into_iter().map(|v| v as u32).collect();
    c.read_paths(section("PATHS")?, budget)?;
    let mut s = section("SPECS")?;
    let n = s.count(0)?;
    let (paths, sets, kinds) = (s.ints(n, 4, budget)?, s.ints(n, 4, budget)?, s.ints(n, 4, budget)?);
    c.build(paths.iter().zip(&sets).zip(&kinds).map(|((p, f), k)| (*p as u32 as usize, *f as u32 as usize, *k)), budget)
}

impl<'a> Crate<'a> {
    fn token(&self, i: u64) -> Result<&str> {
        usize::try_from(i).ok().and_then(|i| self.tokens.get(i)).map(String::as_str).ok_or_else(|| malformed(KIND, format!("token index {i} out of range")))
    }

    fn string(&self, i: u32) -> Result<&str> {
        let t = self.strings.get(i as usize).ok_or_else(|| malformed(KIND, format!("string index {i} out of range")))?;
        self.token(u64::from(*t))
    }

    fn path(&self, i: u32) -> Result<&PathEl> {
        self.paths.get(i as usize).and_then(Option::as_ref).ok_or_else(|| malformed(KIND, format!("path index {i} out of range")))
    }

    fn read_tokens(&mut self, mut s: Cur, budget: &mut Budget) -> Result<()> {
        let n = s.u64()?;
        let (raw_len, comp_len) = (s.u64()?, s.u64()?);
        let raw_len = codec::check_count(raw_len, MAX_ITEMS)?;
        let n = codec::check_count(n, raw_len)?;
        budget.take(raw_len.saturating_add(n.saturating_mul(24)))?;
        let comp = s.bytes(usize::try_from(comp_len).map_err(|_| truncated())?)?;
        let raw = codec::decompress(comp, raw_len)?;
        self.tokens = raw.split(|&c| c == 0).take(n).map(|t| String::from_utf8_lossy(t).into_owned()).collect();
        if self.tokens.len() < n {
            return Err(malformed(KIND, "fewer tokens than declared"));
        }
        Ok(())
    }

    fn read_fields(&mut self, mut section: Cur, budget: &mut Budget) -> Result<()> {
        let count = section.count(0)?;
        let tokens = section.ints(count, 4, budget)?;
        let size = section.u64()?;
        let compressed = section.bytes(usize::try_from(size).map_err(|_| truncated())?)?;
        budget.take(count.saturating_mul(24))?;
        let representations = codec::decompress(compressed, count.saturating_mul(8))?;
        if representations.len() != count * 8 {
            return Err(malformed(KIND, "field values are truncated"));
        }

        self.fields = tokens.iter().zip(representations.as_chunks::<8>().0.iter()).map(|(t, r)| (*t as u32, u64::from_le_bytes(*r))).collect();

        Ok(())
    }

    /// Paths are stored as a pre-order tree: element token (negative for properties) and a
    /// jump per node (-2 leaf, -1 child only, 0 sibling only, >0 child and sibling at +jump).
    fn read_paths(&mut self, mut s: Cur, budget: &mut Budget) -> Result<()> {
        let total = s.count(0)?;
        let n = s.count(0)?;
        let index = s.ints(n, 4, budget)?;
        let elements = s.ints(n, 4, budget)?;
        let jumps = s.ints(n, 4, budget)?;
        budget.take(total.saturating_mul(64))?;
        self.paths = vec![None; total];
        let mut work: Vec<(usize, Option<usize>)> = vec![(0, None)];
        let mut steps = 0usize;
        let bad = || malformed(KIND, "corrupt path table");
        while let Some((mut i, mut parent)) = work.pop() {
            loop {
                steps += 1;
                if steps > n {
                    return Err(bad());
                }
                let this = i;
                i += 1;
                let at = index.get(this).map(|v| *v as u32 as usize).filter(|p| *p < total).ok_or_else(bad)?;
                let el = match parent {
                    None => PathEl { text: "/".into(), parent: None, name: String::new(), prop: false },
                    Some(p) => {
                        let e = *elements.get(this).ok_or_else(bad)?;
                        let name = self.token(e.unsigned_abs())?.to_string();
                        let base = self.paths.get(p).and_then(Option::as_ref).map(|b| b.text.as_str()).ok_or_else(bad)?;
                        let text = match (e < 0, base) {
                            (true, _) => format!("{base}.{name}"),
                            (false, _) if name.starts_with('{') => format!("{base}{name}"),
                            (false, "/") => format!("/{name}"),
                            (false, _) => format!("{base}/{name}"),
                        };
                        budget.take(text.len())?;
                        PathEl { text, parent: Some(p), name, prop: e < 0 }
                    }
                };
                if let Some(slot) = self.paths.get_mut(at) {
                    *slot = Some(el);
                }
                let jump = *jumps.get(this).ok_or_else(bad)?;
                let (child, sibling) = (jump > 0 || jump == -1, jump >= 0);
                if child {
                    if sibling {
                        work.push((this.checked_add(usize::try_from(jump).map_err(|_| bad())?).ok_or_else(bad)?, parent));
                    }
                    parent = Some(at);
                }
                if !child && !sibling {
                    break;
                }
            }
        }
        Ok(())
    }

    /// (name, value rep) of the fields of field set `set`.
    fn fields_of(&self, set: usize) -> Vec<(&str, u64)> {
        self.fieldsets
            .get(set..)
            .unwrap_or_default()
            .iter()
            .take_while(|f| **f != u32::MAX)
            .filter_map(|f| self.fields.get(*f as usize))
            .filter_map(|(t, rep)| Some((self.token(u64::from(*t)).ok()?, *rep)))
            .collect()
    }

    fn build(&self, specs: impl Iterator<Item = (usize, usize, i64)>, budget: &mut Budget) -> Result<Layer> {
        let mut layer = Layer::new();
        let mut prims: HashMap<usize, usize> = HashMap::new();
        for (path, set, kind) in specs {
            let Some(el) = self.paths.get(path).and_then(Option::as_ref) else { return Err(malformed(KIND, format!("spec path {path} out of range"))) };
            let fields = self.fields_of(set);
            match kind {
                SPEC_PSEUDO_ROOT => {
                    prims.insert(path, 0);
                    for (name, rep) in fields {
                        match name {
                            "metersPerUnit" => layer.meters_per_unit = self.number(rep, budget)?,
                            "upAxis" => layer.up_axis = self.tokens_of(rep, budget)?.into_iter().next(),
                            "subLayers" => layer.warn("sublayers are not loaded"),
                            _ => {}
                        }
                    }
                }
                SPEC_PRIM => {
                    let Some(id) = self.prim(&mut layer, &mut prims, path, 0) else { continue };
                    let mut spec = std::mem::take(layer.prims.get_mut(id).ok_or_else(truncated)?);
                    for (name, rep) in fields {
                        match name {
                            "typeName" => spec.type_name = self.tokens_of(rep, budget)?.into_iter().next().unwrap_or_default(),
                            "specifier" => {
                                spec.specifier = match self.value(rep, budget)? {
                                    Some(Val::Num(v)) if v.first() == Some(&1.0) => Specifier::Over,
                                    Some(Val::Num(v)) if v.first() == Some(&2.0) => Specifier::Class,
                                    _ => Specifier::Def,
                                }
                            }
                            "active" => spec.active = self.number(rep, budget)?.map(|v| v != 0.0),
                            "references" => {
                                let (internal, external) = self.references(rep)?;
                                spec.references.extend(internal);
                                if external {
                                    layer.warn("external references and payloads are not loaded");
                                }
                            }
                            "payload" => layer.warn("external references and payloads are not loaded"),
                            _ => {}
                        }
                    }
                    if let Some(slot) = layer.prims.get_mut(id) {
                        *slot = spec;
                    }
                }
                SPEC_ATTRIBUTE | SPEC_RELATIONSHIP => {
                    let Some(owner) = el.parent.filter(|_| el.prop).and_then(|p| self.prim(&mut layer, &mut prims, p, 0)) else { continue };
                    let mut prop = Prop { type_name: if kind == SPEC_RELATIONSHIP { "rel".into() } else { String::new() }, ..Prop::default() };
                    let mut samples = None;
                    for (name, rep) in fields {
                        match name {
                            "typeName" => prop.type_name = self.tokens_of(rep, budget)?.into_iter().next().unwrap_or_default(),
                            "default" | "targetPaths" => prop.value = self.value(rep, budget)?,
                            "connectionPaths" => {
                                if let Some(Val::Path(p)) = self.value(rep, budget)? {
                                    prop.connect = p.into_iter().next();
                                }
                            }
                            "timeSamples" => samples = Some(rep),
                            "interpolation" => prop.interpolation = self.tokens_of(rep, budget)?.into_iter().next(),
                            _ => {}
                        }
                    }
                    if prop.value.is_none()
                        && let Some(rep) = samples
                    {
                        prop.value = self.value(rep, budget)?;
                    }
                    if let Some(spec) = layer.prims.get_mut(owner) {
                        spec.props.insert(el.name.clone(), prop);
                    }
                }
                SPEC_VARIANT | SPEC_VARIANT_SET => layer.warn("variant sets are not applied"),
                _ => {}
            }
        }
        Ok(layer)
    }

    /// The layer prim of prim path `path`, created (with its ancestors) on first use; None for
    /// paths inside variants or nested deeper than [`super::MAX_DEPTH`].
    fn prim(&self, layer: &mut Layer, prims: &mut HashMap<usize, usize>, path: usize, depth: usize) -> Option<usize> {
        if let Some(id) = prims.get(&path) {
            return Some(*id);
        }
        let el = self.paths.get(path)?.as_ref()?;
        if el.prop || el.name.starts_with('{') || depth > super::MAX_DEPTH {
            return None;
        }
        let parent = match el.parent {
            Some(p) if self.paths.get(p)?.as_ref()?.parent.is_none() => 0,
            Some(p) => self.prim(layer, prims, p, depth + 1)?,
            None => return Some(0),
        };
        let id = layer.add_prim(parent, &el.name);
        prims.insert(path, id);
        Some(id)
    }

    fn number(&self, rep: u64, budget: &mut Budget) -> Result<Option<f64>> {
        Ok(match self.value(rep, budget)? {
            Some(Val::Num(v)) => v.first().copied(),
            _ => None,
        })
    }

    fn tokens_of(&self, rep: u64, budget: &mut Budget) -> Result<Vec<String>> {
        Ok(match self.value(rep, budget)? {
            Some(Val::Tok(v)) => v,
            _ => Vec::new(),
        })
    }

    fn at(&self, offset: u64) -> Result<Cur<'a>> {
        let mut c = Cur { b: self.b, pos: 0 };
        c.seek(offset)?;
        Ok(c)
    }

    /// Decode a value rep; None for types a mesh import does not need.
    fn value(&self, rep: u64, budget: &mut Budget) -> Result<Option<Val>> {
        self.value_at_depth(rep, budget, 0)
    }

    fn value_at_depth(&self, rep: u64, budget: &mut Budget, depth: usize) -> Result<Option<Val>> {
        let t = ((rep >> 48) & 0xFF) as u8;
        let payload = rep & codec::PAYLOAD_MASK;
        let (array, inlined, compressed) = (rep & codec::IS_ARRAY != 0, rep & codec::IS_INLINED != 0, rep & codec::IS_COMPRESSED != 0);
        if rep & codec::IS_ARRAY_EDIT != 0 {
            return Ok(None);
        }
        if array && inlined {
            return Ok(Some(Val::Num(Vec::new())));
        }
        let strings = |c: &Crate, ids: Vec<u32>, budget: &mut Budget| -> Result<Vec<String>> {
            let resolve = |i: u32| match t {
                ty::STRING => c.string(i),
                _ => c.token(u64::from(i)),
            };
            ids.into_iter()
                .map(|i| {
                    let s = resolve(i)?.to_string();
                    budget.take(s.len() + 24)?;
                    Ok(s)
                })
                .collect()
        };
        Ok(Some(match t {
            ty::STRING | ty::TOKEN | ty::ASSET_PATH if !array => Val::Tok(strings(self, vec![payload as u32], budget)?),
            ty::STRING | ty::TOKEN | ty::ASSET_PATH => {
                let mut c = self.at(payload)?;
                let ids = self.array_header(&mut c, 4)?;
                Val::Tok(strings(self, (0..ids).map(|_| c.u32()).collect::<Result<_>>()?, budget)?)
            }
            ty::SPECIFIER | ty::VARIABILITY | ty::PERMISSION => Val::Num(vec![f64::from(payload as u32)]),
            ty::TOKEN_VECTOR => {
                let mut c = self.at(payload)?;
                let n = c.count(4)?;
                Val::Tok(strings(self, (0..n).map(|_| c.u32()).collect::<Result<_>>()?, budget)?)
            }
            ty::PATH_VECTOR => {
                let mut c = self.at(payload)?;
                let n = c.count(4)?;
                Val::Path((0..n).map(|_| Ok(self.path(c.u32()?)?.text.clone())).collect::<Result<_>>()?)
            }
            ty::PATH_LIST_OP => Val::Path(self.list_op(payload)?.into_iter().map(|i| Ok(self.path(i)?.text.clone())).collect::<Result<_>>()?),
            ty::TOKEN_LIST_OP => Val::Tok(strings(self, self.list_op(payload)?, budget)?),
            ty::DOUBLE_VECTOR => {
                let mut c = self.at(payload)?;
                let n = c.count(8)?;
                budget.take(n * 8)?;
                Val::Num((0..n).map(|_| c.f64()).collect::<Result<_>>()?)
            }
            ty::TIME_SAMPLES if depth == 0 => {
                let mut c = self.at(payload)?;
                c.jump()?;
                c.u64()?;
                c.jump()?;
                if c.u64()? == 0 {
                    return Ok(None);
                }
                return self.value_at_depth(c.u64()?, budget, 1);
            }
            _ => match codec::numeric(t) {
                Some((s, comps)) if inlined => Val::Num(inline_numeric(t, s, comps, payload)),
                Some((s, comps)) if array => self.numeric_array(t, s, comps, payload, compressed, budget)?,
                Some((s, comps)) => {
                    let mut c = self.at(payload)?;
                    let raw = c.bytes(s.size() * comps)?;
                    let mut v: Vec<f64> = raw.chunks_exact(s.size()).map(|b| scalar(s, b)).collect();
                    quat_order(t, &mut v);
                    Val::Num(v)
                }
                None => return Ok(None),
            },
        }))
    }

    /// Element count of an array at `c` (`elem` bytes each), past the pre-0.5 shape word.
    fn array_header(&self, c: &mut Cur, elem: usize) -> Result<usize> {
        if self.ver < (0, 5, 0) {
            c.u32()?;
        }
        let n = if self.ver < (0, 7, 0) { u64::from(c.u32()?) } else { c.u64()? };
        codec::check_count(n, c.cap(elem))
    }

    fn numeric_array(&self, t: u8, s: Scalar, comps: usize, payload: u64, compressed: bool, budget: &mut Budget) -> Result<Val> {
        let mut c = self.at(payload)?;
        let int = matches!(s, Scalar::I32 | Scalar::U32 | Scalar::I64 | Scalar::U64) && comps == 1 && self.ver >= (0, 5, 0);
        let float = matches!(s, Scalar::F16 | Scalar::F32 | Scalar::F64) && comps == 1 && self.ver >= (0, 6, 0);
        let packed = compressed && (int || float);
        let n = self.array_header(&mut c, if packed { 0 } else { s.size() * comps })?;
        budget.take(n.saturating_mul(comps).saturating_mul(8))?;
        if packed && n >= codec::MIN_COMPRESSED_ARRAY {
            let as_f64 = |v: i64| match s {
                Scalar::U32 => f64::from(v as u32),
                Scalar::U64 => v as u64 as f64,
                _ => v as f64,
            };
            if int {
                let width = s.size();
                return Ok(Val::Num(c.ints(n, width, budget)?.into_iter().map(as_f64).collect()));
            }
            return Ok(Val::Num(match c.u8()? {
                b'i' => c.ints(n, 4, budget)?.into_iter().map(|v| v as f64).collect(),
                b't' => {
                    let lut_len = c.u32()? as usize;
                    let lut: Vec<f64> = c.bytes(lut_len.checked_mul(s.size()).ok_or_else(truncated)?)?.chunks_exact(s.size()).map(|b| scalar(s, b)).collect();
                    let idx = c.ints(n, 4, budget)?;
                    idx.into_iter()
                        .map(|i| lut.get(i as u32 as usize).copied().ok_or_else(|| malformed(KIND, "lookup table index out of range")))
                        .collect::<Result<_>>()?
                }
                code => return Err(malformed(KIND, format!("unknown float array coding {code}"))),
            }));
        }
        let raw = c.bytes(n.checked_mul(comps).and_then(|k| k.checked_mul(s.size())).ok_or_else(truncated)?)?;
        let mut v: Vec<f64> = raw.chunks_exact(s.size()).map(|b| scalar(s, b)).collect();
        quat_order(t, &mut v);
        Ok(Val::Num(v))
    }

    /// The kept items (explicit, added, prepended, appended) of a token or path list op.
    fn list_op(&self, payload: u64) -> Result<Vec<u32>> {
        let mut c = self.at(payload)?;
        let header = c.u8()?;
        let mut out = Vec::new();
        // Lists in file order: explicit, added, prepended, appended, deleted, ordered.
        for bit in [1, 2, 5, 6, 3, 4] {
            if header & (1 << bit) == 0 {
                continue;
            }
            let n = c.count(4)?;
            let items: Vec<u32> = (0..n).map(|_| c.u32()).collect::<Result<_>>()?;
            if bit <= 2 || bit >= 5 {
                out.extend(items);
            }
        }
        Ok(out)
    }

    /// Internal reference targets of a reference list op, and whether any was external.
    fn references(&self, rep: u64) -> Result<(Vec<String>, bool)> {
        if ((rep >> 48) & 0xFF) as u8 != ty::REFERENCE_LIST_OP || rep & codec::IS_INLINED != 0 {
            return Ok((Vec::new(), false));
        }
        let mut c = self.at(rep & codec::PAYLOAD_MASK)?;
        let header = c.u8()?;
        let (mut internal, mut external) = (Vec::new(), false);
        for bit in [1, 2, 5, 6, 3, 4] {
            if header & (1 << bit) == 0 {
                continue;
            }
            let n = c.count(28)?;
            for _ in 0..n {
                let asset = self.string(c.u32()?)?.to_string();
                let prim = self.path(c.u32()?)?.text.clone();
                c.f64()?;
                c.f64()?;
                // customData: string keys with nested values, skipped.
                let entries = c.count(12)?;
                for _ in 0..entries {
                    c.u32()?;
                    c.jump()?;
                    c.u64()?;
                }
                if bit == 3 || bit == 4 {
                    continue;
                }
                match (asset.is_empty(), prim.as_str()) {
                    (true, p) if p != "/" && !p.is_empty() => internal.push(prim),
                    _ => external = true,
                }
            }
        }
        Ok((internal, external))
    }
}

/// A numeric value stored in the rep itself: 32-bit scalars (doubles as floats, 64-bit ints as
/// 32-bit), vectors as int8 components and matrices as int8 diagonals.
fn inline_numeric(t: u8, s: Scalar, comps: usize, payload: u64) -> Vec<f64> {
    let bytes = payload.to_le_bytes();
    let i8s = |n: usize| bytes.iter().take(n).map(|b| f64::from(*b as i8)).collect::<Vec<f64>>();
    match (t, comps) {
        (ty::MATRIX2D..=ty::MATRIX4D, _) => {
            let dim = usize::from(t - ty::MATRIX2D) + 2;
            let diag = i8s(dim);
            (0..dim * dim).map(|k| if k % (dim + 1) == 0 { diag.get(k / (dim + 1)).copied().unwrap_or(1.0) } else { 0.0 }).collect()
        }
        (_, 1) => vec![match s {
            Scalar::U8 => f64::from(payload as u8),
            Scalar::I32 | Scalar::I64 => f64::from(payload as u32 as i32),
            Scalar::U32 | Scalar::U64 => f64::from(payload as u32),
            Scalar::F16 => f64::from(half::f16::from_bits(payload as u16).to_f32()),
            Scalar::F32 | Scalar::F64 => f64::from(f32::from_bits(payload as u32)),
        }],
        _ => i8s(comps),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn crate_over(b: &[u8], ver: (u8, u8, u8)) -> Crate<'_> {
        Crate { b, ver, tokens: Vec::new(), strings: Vec::new(), fields: Vec::new(), fieldsets: Vec::new(), paths: Vec::new() }
    }

    fn compressed_ints(out: &mut Vec<u8>, v: &[i64]) {
        let comp = codec::compress(&codec::encode_ints(v, 4));
        out.extend_from_slice(&(comp.len() as u64).to_le_bytes());
        out.extend_from_slice(&comp);
    }

    #[test]
    fn compressed_float_and_int_arrays() {
        let n = 20usize;
        let mut b = Vec::new();
        // 'i': integral floats as compressed ints.
        b.extend_from_slice(&(n as u64).to_le_bytes());
        b.push(b'i');
        let ints: Vec<i64> = (0..n as i64).map(|i| i * 3 - 7).collect();
        compressed_ints(&mut b, &ints);
        let lut_at = b.len() as u64;
        // 't': a lookup table of doubles and compressed indexes into it.
        b.extend_from_slice(&(n as u64).to_le_bytes());
        b.push(b't');
        b.extend_from_slice(&2u32.to_le_bytes());
        b.extend_from_slice(&0.25f64.to_le_bytes());
        b.extend_from_slice(&(-8.5f64).to_le_bytes());
        let idx: Vec<i64> = (0..n as i64).map(|i| i % 2).collect();
        compressed_ints(&mut b, &idx);
        let i64_at = b.len() as u64;
        b.extend_from_slice(&(n as u64).to_le_bytes());
        let comp = codec::compress(&codec::encode_ints(&(0..n as i64).map(|i| i << 33).collect::<Vec<_>>(), 8));
        b.extend_from_slice(&(comp.len() as u64).to_le_bytes());
        b.extend_from_slice(&comp);
        let c = crate_over(&b, (0, 8, 0));
        let mut budget = Budget::new();
        let f32s = c.numeric_array(ty::FLOAT, Scalar::F32, 1, 0, true, &mut budget).unwrap();
        assert_eq!(f32s, Val::Num(ints.iter().map(|&i| i as f64).collect()));
        let lut = c.numeric_array(ty::DOUBLE, Scalar::F64, 1, lut_at, true, &mut budget).unwrap();
        assert_eq!(lut, Val::Num((0..n).map(|i| if i % 2 == 0 { 0.25 } else { -8.5 }).collect()));
        let wide = c.numeric_array(ty::INT64, Scalar::I64, 1, i64_at, true, &mut budget).unwrap();
        assert_eq!(wide, Val::Num((0..n as i64).map(|i| (i << 33) as f64).collect()));
        // Out-of-range lookups and unknown codings are errors.
        let mut bad = b.clone();
        bad[lut_at as usize + 8] = b'x';
        assert!(crate_over(&bad, (0, 8, 0)).numeric_array(ty::DOUBLE, Scalar::F64, 1, lut_at, true, &mut budget).is_err());
    }

    #[test]
    fn old_array_headers_halves_quats_and_inlined_values() {
        // 0.4: discarded shape word and a u32 count; half floats.
        let mut b = Vec::new();
        b.extend_from_slice(&1u32.to_le_bytes());
        b.extend_from_slice(&2u32.to_le_bytes());
        for v in [1.5f32, -2.0] {
            b.extend_from_slice(&half::f16::from_f32(v).to_bits().to_le_bytes());
        }
        let quat_at = b.len() as u64;
        for v in [0.0f32, 0.0, 1.0, 0.5] {
            b.extend_from_slice(&v.to_le_bytes());
        }
        let c = crate_over(&b, (0, 4, 0));
        let mut budget = Budget::new();
        assert_eq!(c.numeric_array(ty::HALF, Scalar::F16, 1, 0, false, &mut budget).unwrap(), Val::Num(vec![1.5, -2.0]));
        let quat = c.value((17u64 << 48) | quat_at, &mut budget).unwrap();
        assert_eq!(quat, Some(Val::Num(vec![0.5, 0.0, 0.0, 1.0])), "quaternions read real first");
        assert_eq!(inline_numeric(ty::MATRIX2D, Scalar::F64, 4, 0x0302), vec![2.0, 0.0, 0.0, 3.0]);
        assert_eq!(inline_numeric(24, Scalar::F32, 3, 0x00FF_0201), vec![1.0, 2.0, -1.0]);
        assert_eq!(inline_numeric(ty::DOUBLE, Scalar::F64, 1, u64::from(0.5f32.to_bits())), vec![0.5]);
        assert_eq!(inline_numeric(ty::INT, Scalar::I32, 1, u64::from((-3i32) as u32)), vec![-3.0]);
        // Array edits are not applied; empty arrays are inlined.
        assert_eq!(c.value(codec::IS_ARRAY_EDIT | (3u64 << 48), &mut budget).unwrap(), None);
        assert_eq!(c.value(codec::IS_ARRAY | codec::IS_INLINED | (3u64 << 48), &mut budget).unwrap(), Some(Val::Num(vec![])));
    }
}
