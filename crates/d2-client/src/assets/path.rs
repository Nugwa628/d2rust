//! Client paths use the shared file-source seam.
pub use d2_server::file_source::*;

#[cfg(test)]
mod tests {
    #[test]
    fn two_spellings_one_asset_handle() {
        assert_eq!(
            super::super::asset_path(r"DATA\Global\X.DC6"),
            super::super::asset_path(r"data\global\x.dc6")
        );
    }
}
