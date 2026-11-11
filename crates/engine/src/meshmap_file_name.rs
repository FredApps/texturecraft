//! Which texture set and mesh map a baked map file is for, by file name (`Body_ao.1001.png` = Body's ambient occlusion)

use std::path::Path;

use texturecraft_core::{BakedMap, UDIM_RANGE};

/// Mesh maps named by two words anywhere in the file name; they win over the one-word names
const TWO_WORD_NAMES: &[(&str, &[&str], BakedMap)] = &[
    ("world", &["normal", "normals"], BakedMap::WorldNormal),
    ("bent", &["normal", "normals"], BakedMap::BentNormal),
    ("ambient", &["occlusion"], BakedMap::AmbientOcclusion),
];

/// Mesh maps named by one word, including the two-word names written as one
const ONE_WORD_NAMES: &[(&str, BakedMap)] = &[
    ("worldnormal", BakedMap::WorldNormal),
    ("worldnormals", BakedMap::WorldNormal),
    ("bentnormal", BakedMap::BentNormal),
    ("bentnormals", BakedMap::BentNormal),
    ("ambientocclusion", BakedMap::AmbientOcclusion),
    ("normal", BakedMap::Normal),
    ("normals", BakedMap::Normal),
    ("nrm", BakedMap::Normal),
    ("occlusion", BakedMap::AmbientOcclusion),
    ("ao", BakedMap::AmbientOcclusion),
    ("curvature", BakedMap::Curvature),
    ("curv", BakedMap::Curvature),
    ("id", BakedMap::Id),
    ("position", BakedMap::Position),
    ("pos", BakedMap::Position),
    ("thickness", BakedMap::Thickness),
    ("height", BakedMap::Height),
    ("displacement", BakedMap::Height),
    ("opacity", BakedMap::Opacity),
    ("alpha", BakedMap::Opacity),
];

fn is_separator(c: char) -> bool {
    matches!(c, '_' | '-' | '.' | ' ')
}

/// The lowercase file name without its folder and extension
fn lowercase_stem(path: &str) -> String {
    Path::new(path).file_stem().and_then(|stem| stem.to_str()).unwrap_or_default().to_lowercase()
}

/// What follows `set_name` at the start of a lowercase stem, when a separator follows it
fn after_set_name<'a>(stem: &'a str, set_name: &str) -> Option<&'a str> {
    if set_name.is_empty() {
        return None;
    }

    stem.strip_prefix(&set_name.to_lowercase()).filter(|rest| rest.starts_with(is_separator))
}

/// The mesh map a file holds, from the words of its name after `set_name`; the last recognised word wins
pub(crate) fn baked_map_from_file_name(path: &str, set_name: &str) -> Option<BakedMap> {
    let stem = lowercase_stem(path);
    let rest = after_set_name(&stem, set_name).unwrap_or(&stem);
    let is_tile_number = |word: &str| word.len() == 4 && word.parse::<u32>().is_ok_and(|number| UDIM_RANGE.contains(&number));
    let words: Vec<&str> = rest.split(is_separator).filter(|word| !word.is_empty() && !is_tile_number(word)).collect();

    let has = |name: &str| words.contains(&name);
    TWO_WORD_NAMES
        .iter()
        .find(|(first, seconds, _)| has(first) && seconds.iter().any(|second| has(second)))
        .map(|(_, _, map)| *map)
        .or_else(|| words.iter().rev().find_map(|word| ONE_WORD_NAMES.iter().find(|(name, _)| name == word).map(|(_, map)| *map)))
}

/// The set a file is for: the longest set name its name starts with before a separator (any name with one set)
pub(crate) fn texture_set_from_file_name(path: &str, set_names: &[&str]) -> Option<usize> {
    if set_names.len() == 1 {
        return Some(0);
    }

    let stem = lowercase_stem(path);
    set_names.iter().enumerate().filter(|(_, name)| after_set_name(&stem, name).is_some()).max_by_key(|(_, name)| name.chars().count()).map(|(ts, _)| ts)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_word_names_its_map() {
        let cases = [
            ("Body_normal.png", BakedMap::Normal),
            ("Body_normals.png", BakedMap::Normal),
            ("Body_nrm.png", BakedMap::Normal),
            ("Body_world_normal.png", BakedMap::WorldNormal),
            ("Body_normals_world.png", BakedMap::WorldNormal),
            ("Body_WorldNormal.png", BakedMap::WorldNormal),
            ("Body_bent_normals.png", BakedMap::BentNormal),
            ("Body-ambient-occlusion.png", BakedMap::AmbientOcclusion),
            ("Body_AmbientOcclusion.png", BakedMap::AmbientOcclusion),
            ("Body_occlusion.png", BakedMap::AmbientOcclusion),
            ("Body AO.png", BakedMap::AmbientOcclusion),
            ("Body_curvature.png", BakedMap::Curvature),
            ("Body_curv.png", BakedMap::Curvature),
            ("Body_ID.png", BakedMap::Id),
            ("Body_position.png", BakedMap::Position),
            ("Body_pos.png", BakedMap::Position),
            ("Body_thickness.png", BakedMap::Thickness),
            ("Body_height.png", BakedMap::Height),
            ("Body_displacement.png", BakedMap::Height),
            ("Body_opacity.png", BakedMap::Opacity),
            ("Body_alpha.png", BakedMap::Opacity),
        ];
        for (file, map) in cases {
            assert_eq!(baked_map_from_file_name(file, "Body"), Some(map), "{file}");
        }
    }

    #[test]
    fn tile_numbers_and_folders_are_ignored() {
        assert_eq!(baked_map_from_file_name("/bakes/Body_ao.1001.png", "Body"), Some(BakedMap::AmbientOcclusion));
        assert_eq!(baked_map_from_file_name("Body_normal_1002.tga", "Body"), Some(BakedMap::Normal));
        assert_eq!(texture_set_from_file_name("/bakes/Body_ao.1001.png", &["Head", "Body"]), Some(1));
    }

    #[test]
    fn unknown_words_match_no_map() {
        assert_eq!(baked_map_from_file_name("Body_diffuse.png", "Body"), None);
        assert_eq!(baked_map_from_file_name("Body_1001.png", "Body"), None);
        assert_eq!(baked_map_from_file_name("ambient.png", "Body"), None);
        assert_eq!(baked_map_from_file_name("", "Body"), None);
    }

    #[test]
    fn the_set_name_is_not_read_as_a_map() {
        assert_eq!(baked_map_from_file_name("World_normal.png", "World"), Some(BakedMap::Normal));
        assert_eq!(baked_map_from_file_name("ID_diffuse.png", "ID"), None);
        assert_eq!(baked_map_from_file_name("Body_ao.png", ""), Some(BakedMap::AmbientOcclusion));
    }

    #[test]
    fn the_longest_set_name_wins() {
        let sets = ["Body", "Body_Upper", "Head"];
        assert_eq!(texture_set_from_file_name("Body_Upper_ao.png", &sets), Some(1));
        assert_eq!(texture_set_from_file_name("body_ao.png", &sets), Some(0));
        assert_eq!(texture_set_from_file_name("HEAD-normal.png", &sets), Some(2));
    }

    #[test]
    fn a_set_name_must_end_at_a_separator() {
        let sets = ["Body", "Head"];
        assert_eq!(texture_set_from_file_name("Bodywork_ao.png", &sets), None);
        assert_eq!(texture_set_from_file_name("Arm_ao.png", &sets), None);
        assert_eq!(texture_set_from_file_name("Body.png", &sets), None);
    }

    #[test]
    fn one_set_takes_every_file() {
        assert_eq!(texture_set_from_file_name("ao.png", &["Body"]), Some(0));
        assert_eq!(texture_set_from_file_name("Head_ao.png", &["Body"]), Some(0));
        assert_eq!(texture_set_from_file_name("ao.png", &[]), None);
    }
}
