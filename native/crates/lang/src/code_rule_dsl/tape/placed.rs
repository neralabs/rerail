//! A loaded tape's one home (decision 0353 step 4a): the placer is the slabs' (`rerail_slabs::placed`, moved there in
//! decision 0686, B4, so a crate below this one places a read as the engine does); a document's source read into it,
//! and a source in hand housed there, are this crate's.

#[cfg(any(target_os = "macos", target_os = "linux", windows))]
pub use rerail_slabs::placed::{HomeOf, device, home_of, place, placer, slab_room};

#[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
pub use self::no_device::{HomeOf, home_of, place, placer};

/// Where there is no device layer: every home a vector of its own.
#[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
mod no_device {
    use rerail_tape_format::home::{Home, Room};

    pub struct HomeOf {
        pub id: usize,
        pub host: u64,
        pub at: u64,
    }

    pub fn home_of(_: Option<&std::sync::Arc<dyn Room>>, bytes: &[u8]) -> HomeOf {
        let host = bytes.as_ptr() as usize;
        HomeOf { id: host, host: host as u64, at: 0 }
    }

    pub fn place(len: usize, fill: impl FnOnce(&mut [u8]) -> std::io::Result<()>) -> std::io::Result<Home> {
        let mut v = rerail_tape_format::Vector::with_capacity(len);
        v.resize(len, 0);
        fill(&mut v)?;
        Ok(Home::Own(v))
    }

    pub fn placer(len: usize, fill: &mut dyn FnMut(&mut [u8]) -> std::io::Result<()>) -> std::io::Result<Home> {
        place(len, fill)
    }
}

/// A document's source read from `path` straight into its home (decision 0353 step 7, row 13): a slab's room where
/// the device's pool gives one, which every reader then reads where it lies, the device too. Nothing is read twice.
pub fn read_text(path: &std::path::Path) -> std::io::Result<crate::code_rule_dsl::tape::Text> {
    let home = rerail_observe::git_workspace::snapshot::read_placed(path, &placer)?;
    crate::code_rule_dsl::tape::Text::new(home).map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
}

/// `source` where a device reads it: as it is where it lies in a room already (a loader's, `read_text`) or where
/// there is no device; else written into a room of its own, which the document that takes it reads (the host and the
/// device alike), and the residency takes as it lies (decision 0353 step 7, row 13).
pub fn housed(source: std::sync::Arc<crate::code_rule_dsl::tape::Text>) -> std::sync::Arc<crate::code_rule_dsl::tape::Text> {
    #[cfg(any(target_os = "macos", target_os = "linux", windows))]
    if !source.home().placed() && device().is_some() {
        // copy (0353 step 4a): a source made in memory (a test's literal, an edit's text, a store's read into a
        // vector) written into its room, as a parse's tape is (`place_stored`). It goes when its maker writes it there.
        const INTO_ROOM: rerail_prof::copies::Site = rerail_prof::copies::Site::new(17, "4a", "a source in hand written into its room (placed::housed)");
        let len = source.len();
        let home = INTO_ROOM.time(|| {
            (
                place(len, |out| {
                    // copy (0353 step 4a): the source (row 17).
                    out.copy_from_slice(source.as_bytes());
                    Ok(())
                }),
                len,
            )
        });
        if let Ok(home) = home
            && home.placed()
            && let Ok(text) = crate::code_rule_dsl::tape::Text::new(home)
        {
            return std::sync::Arc::new(text);
        }
    }
    source
}
