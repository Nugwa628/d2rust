//! Coverage tests (c2-ui-render session).
use super::*;

#[test]
fn direction_source_by_unit_type() {
    for t in [0, 1, 3] {
        assert_eq!(dir_source(t), Some(DirSource::DynamicPath));
    }
    for t in [2, 4] {
        assert_eq!(dir_source(t), Some(DirSource::StaticPathByte));
    }
    assert_eq!(dir_source(5), None);
}
