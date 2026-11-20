//! The global registry of source files.

use std::fmt::Write as _;
use std::ops::Range;
use std::sync::{Arc, RwLock, RwLockReadGuard, RwLockWriteGuard};

use seal_index::{Idx, newtype_index};

use crate::error::{SourceMapError, SpanError};
use crate::file::SourceFile;
use crate::file_name::FileName;
use crate::line_index::LineCol;
use crate::remap::PathRemapper;
use crate::span::{BytePos, Span};

newtype_index! {
    /// Identifies a file registered in a [`SourceMap`].
    pub struct FileId;
}

/// Maximum accepted file size in bytes.
///
/// Global positions are `u32`; leaving headroom keeps arithmetic and the one-byte
/// inter-file gap from overflowing.
pub const MAX_FILE_SIZE: u32 = u32::MAX - 2;

/// A set of registered source files and the position space that spans refer to.
///
/// The map is shareable and thread-safe: phases borrow it immutably while
/// diagnostics render from it, and macro expansion (or a language server) may
/// register more files behind `&self`.
pub struct SourceMap {
    files: RwLock<Vec<Arc<SourceFile>>>,
    remapper: RwLock<PathRemapper>,
    max_file_size: u32,
}

/// The portion of a source file covered by a span, kept alongside the file so
/// callers do not need a second lookup.
#[derive(Clone, Debug)]
pub struct Snippet {
    file: Arc<SourceFile>,
    range: Range<u32>,
}

impl Snippet {
    /// Returns the covered text.
    #[inline]
    pub fn text(&self) -> &str {
        let start = self.range.start as usize;
        let end = self.range.end as usize;
        &self.file.text()[start..end]
    }

    /// Returns the zero-based line where the snippet starts.
    #[inline]
    pub fn start_line(&self) -> u32 {
        self.file.lines().line_of(self.range.start)
    }

    /// Returns the zero-based line where the snippet ends.
    #[inline]
    pub fn end_line(&self) -> u32 {
        let end = self.range.end.saturating_sub(1).max(self.range.start);
        self.file.lines().line_of(end)
    }

    /// Returns the file containing the snippet.
    #[inline]
    pub fn file(&self) -> &Arc<SourceFile> {
        &self.file
    }

    /// Returns the snippet's file-local byte range.
    #[inline]
    pub fn range(&self) -> Range<u32> {
        self.range.clone()
    }
}

impl Default for SourceMap {
    fn default() -> Self {
        Self::new()
    }
}

impl SourceMap {
    /// Creates an empty source map with no path remapping.
    pub fn new() -> Self {
        Self::with_max_file_size(MAX_FILE_SIZE)
    }

    /// Creates an empty source map with an explicit per-file size limit.
    ///
    /// The limit exists for tests and for embedders with tighter memory budgets;
    /// the maximum the position space can represent is [`MAX_FILE_SIZE`].
    pub fn with_max_file_size(max_file_size: u32) -> Self {
        Self {
            files: RwLock::new(Vec::new()),
            remapper: RwLock::new(PathRemapper::new()),
            max_file_size: max_file_size.min(MAX_FILE_SIZE),
        }
    }

    /// Replaces the path remapper used when displaying file names.
    pub fn set_remapper(&self, remapper: PathRemapper) {
        *self.write_remapper() = remapper;
    }

    /// Registers a file and returns its id.
    ///
    /// Registering the same [`FileName`] twice returns the existing id; the text
    /// of the second registration is ignored. This keeps diagnostics pointing at
    /// one file id even when a module is reached by two paths.
    pub fn add_file(&self, name: FileName, source: String) -> Result<FileId, SourceMapError> {
        self.add_file_impl(name, source, true)
    }

    /// Registers a file and always returns a fresh id.
    ///
    /// The same source can legitimately be compiled as more than one package
    /// instance. Those instances need separate spans even though diagnostics
    /// should still display the same real path, so this method skips the
    /// duplicate-name check in [`SourceMap::add_file`].
    pub fn add_file_distinct(
        &self,
        name: FileName,
        source: String,
    ) -> Result<FileId, SourceMapError> {
        self.add_file_impl(name, source, false)
    }

    fn add_file_impl(
        &self,
        name: FileName,
        source: String,
        deduplicate: bool,
    ) -> Result<FileId, SourceMapError> {
        let mut files = self.write_files();
        if deduplicate {
            if let Some(index) = files.iter().position(|file| file.name() == &name) {
                return Ok(FileId::from_usize(index));
            }
        }
        if files.len() >= FileId::MAX.index() as usize {
            return Err(SourceMapError::TooManyFiles);
        }

        let (text, has_bom) = match source.strip_prefix('\u{feff}') {
            Some(rest) => (rest.to_owned(), true),
            None => (source, false),
        };
        let len = u32::try_from(text.len()).map_err(|_| SourceMapError::FileTooLarge {
            len: text.len() as u64,
            max: MAX_FILE_SIZE,
        })?;
        if len > self.max_file_size {
            return Err(SourceMapError::FileTooLarge {
                len: u64::from(len),
                max: self.max_file_size,
            });
        }

        let start = match files.last() {
            // One-byte gap between files: a span that crosses the gap is
            // rejected by `file_of` instead of silently attributing the wrong
            // file.
            Some(last) => last.end().to_u32().saturating_add(1),
            None => 0,
        };
        let end = start
            .checked_add(len)
            .ok_or(SourceMapError::PositionOverflow { start, len })?;

        let file = Arc::new(SourceFile::new(
            name,
            text,
            BytePos::from_u32(start),
            has_bom,
        ));
        debug_assert_eq!(file.end().to_u32(), end);
        files.push(Arc::clone(&file));
        Ok(FileId::from_usize(files.len() - 1))
    }

    /// Returns the file registered under `id`.
    pub fn file(&self, id: FileId) -> Option<Arc<SourceFile>> {
        self.read_files().get(id.as_usize()).cloned()
    }

    /// Returns all registered files with their ids, in registration order.
    pub fn files(&self) -> Vec<(FileId, Arc<SourceFile>)> {
        self.read_files()
            .iter()
            .enumerate()
            .map(|(index, file)| (FileId::from_usize(index), Arc::clone(file)))
            .collect()
    }

    /// Returns the number of registered files.
    pub fn len(&self) -> usize {
        self.read_files().len()
    }

    /// Returns `true` if no file is registered.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Creates a span from a file-local byte range.
    pub fn span(&self, file: FileId, range: Range<u32>) -> Result<Span, SpanError> {
        let file = self.file(file).ok_or(SpanError::UnknownFile)?;
        file.span(range)
    }

    /// Returns the empty span at the start of `file`.
    pub fn empty_span(&self, file: FileId) -> Result<Span, SpanError> {
        Ok(self.file(file).ok_or(SpanError::UnknownFile)?.empty_span())
    }

    /// Returns the span covering all of `file`.
    pub fn full_span(&self, file: FileId) -> Result<Span, SpanError> {
        Ok(self.file(file).ok_or(SpanError::UnknownFile)?.full_span())
    }

    /// Returns the file that fully contains `span`.
    ///
    /// Returns `None` when the span crosses the inter-file gap, which can only
    /// happen if a span was built without validation.
    pub fn file_of(&self, span: Span) -> Option<FileId> {
        let files = self.read_files();
        let index = files.partition_point(|file| file.end().to_u32() < span.lo().to_u32());
        let file = files.get(index)?;
        if file.contains_span(span) {
            Some(FileId::from_usize(index))
        } else {
            None
        }
    }

    /// Finds the file containing a global position.
    pub fn lookup(&self, position: BytePos) -> Option<(FileId, Arc<SourceFile>)> {
        let files = self.read_files();
        let index = files.partition_point(|file| file.end().to_u32() < position.to_u32());
        let file = files.get(index)?;
        if file.contains(position) {
            Some((FileId::from_usize(index), Arc::clone(file)))
        } else {
            None
        }
    }

    /// Returns the snippet covered by `span`.
    pub fn snippet(&self, span: Span) -> Option<Snippet> {
        let id = self.file_of(span)?;
        let file = self.file(id)?;
        let start = file.local_offset(span.lo())?;
        let end = file.local_offset(span.hi())?;
        Some(Snippet {
            file,
            range: start..end,
        })
    }

    /// Returns the one-based start and end positions of `span`.
    pub fn line_col(&self, span: Span) -> Option<(FileId, LineCol, LineCol)> {
        let id = self.file_of(span)?;
        let file = self.file(id)?;
        let start = file.line_col(span.lo()).ok()?;
        let end = file.line_col(span.hi()).ok()?;
        Some((id, start, end))
    }

    /// Applies the configured path remapping to an arbitrary path.
    ///
    /// Used for diagnostics about files that were never registered (for example
    /// a failed read), so remapping stays consistent across all messages.
    pub fn remap_path(&self, path: &std::path::Path) -> std::path::PathBuf {
        self.read_remapper().remap(path)
    }

    /// Returns the displayed path of `id`, applying the configured remapping.
    pub fn display_name(&self, id: FileId) -> Option<String> {
        let file = self.file(id)?;
        Some(match file.name() {
            FileName::Real(path) => self.read_remapper().remap(path).display().to_string(),
            FileName::Virtual(name) => name.clone(),
        })
    }

    /// Formats `span` as `path:line:column`.
    pub fn span_to_string(&self, span: Span) -> Option<String> {
        let (id, start, _) = self.line_col(span)?;
        let mut output = self.display_name(id)?;
        let _ = write!(&mut output, ":{}:{}", start.line, start.column);
        Some(output)
    }

    fn read_files(&self) -> RwLockReadGuard<'_, Vec<Arc<SourceFile>>> {
        self.files.read().unwrap_or_else(|error| error.into_inner())
    }

    fn write_files(&self) -> RwLockWriteGuard<'_, Vec<Arc<SourceFile>>> {
        self.files
            .write()
            .unwrap_or_else(|error| error.into_inner())
    }

    fn read_remapper(&self) -> RwLockReadGuard<'_, PathRemapper> {
        self.remapper
            .read()
            .unwrap_or_else(|error| error.into_inner())
    }

    fn write_remapper(&self) -> RwLockWriteGuard<'_, PathRemapper> {
        self.remapper
            .write()
            .unwrap_or_else(|error| error.into_inner())
    }
}

impl std::fmt::Debug for SourceMap {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SourceMap")
            .field("files", &self.len())
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::{FileId, SourceMap, SourceMapError};
    use crate::error::SpanError;
    use crate::file_name::FileName;
    use crate::remap::PathRemapper;
    use crate::span::BytePos;
    use seal_index::Idx;

    fn map() -> SourceMap {
        SourceMap::new()
    }

    fn add(map: &SourceMap, name: &str, text: &str) -> FileId {
        map.add_file(FileName::virtual_name(name), text.to_owned())
            .expect("file registers")
    }

    #[test]
    fn file_ids_are_dense_and_stable() {
        let map = map();
        let first = add(&map, "a", "one");
        let second = add(&map, "b", "two");
        assert_eq!(first.index(), 0);
        assert_eq!(second.index(), 1);
        assert_eq!(map.len(), 2);
        assert!(!map.is_empty());
        assert_eq!(map.file(first).expect("file exists").text(), "one");
        assert_eq!(map.file(second).expect("file exists").text(), "two");
    }

    #[test]
    fn duplicate_names_reuse_the_existing_id() {
        let map = map();
        let first = add(&map, "a", "one");
        let second = add(&map, "a", "different text");
        assert_eq!(first, second);
        assert_eq!(map.len(), 1);
        assert_eq!(map.file(first).expect("file exists").text(), "one");
    }

    #[test]
    fn distinct_files_can_share_a_display_name() {
        let map = map();
        let first = map
            .add_file_distinct(FileName::virtual_name("a"), "one".to_owned())
            .expect("first file");
        let second = map
            .add_file_distinct(FileName::virtual_name("a"), "two".to_owned())
            .expect("second file");
        assert_ne!(first, second);
        assert_eq!(map.len(), 2);
        assert_eq!(map.file(second).expect("file exists").text(), "two");
    }

    #[test]
    fn bom_is_stripped_and_recorded() {
        let map = map();
        let id = add(&map, "bom", "\u{feff}x");
        let file = map.file(id).expect("file exists");
        assert!(file.has_bom());
        assert_eq!(file.text(), "x");
        assert_eq!(file.len(), 1);
    }

    #[test]
    fn spans_are_validated() {
        let map = map();
        let id = add(&map, "a", "abc");
        let span = map.span(id, 0..3).expect("valid span");
        assert_eq!(span.len(), 3);
        assert_eq!(
            map.span(id, 0..4),
            Err(SpanError::OutOfBounds { end: 4, len: 3 })
        );
        assert_eq!(
            map.span(FileId::from_usize(9), 0..1),
            Err(SpanError::UnknownFile)
        );
    }

    #[test]
    fn snippet_and_positions() {
        let map = map();
        let id = add(&map, "a", "let x = 1;\nlet y = 2;\n");
        let span = map.span(id, 11..20).expect("valid span");
        let snippet = map.snippet(span).expect("span has a snippet");
        assert_eq!(snippet.text(), "let y = 2");
        assert_eq!(snippet.start_line(), 1);
        assert_eq!(snippet.end_line(), 1);
        let (_, start, end) = map.line_col(span).expect("span has positions");
        assert_eq!((start.line, start.column), (2, 1));
        assert_eq!((end.line, end.column), (2, 10));
    }

    #[test]
    fn span_to_string_includes_file_and_position() {
        let map = map();
        let id = map
            .add_file(
                FileName::real("/home/user/project/main.sl"),
                "func".to_owned(),
            )
            .expect("file registers");
        let span = map.span(id, 0..4).expect("valid span");
        assert_eq!(
            map.span_to_string(span).as_deref(),
            Some("/home/user/project/main.sl:1:1")
        );
        map.set_remapper(
            PathRemapper::from_specs(["/home/user/project=/src"]).expect("valid remap"),
        );
        assert_eq!(
            map.span_to_string(span).as_deref(),
            Some("/src/main.sl:1:1")
        );
    }

    #[test]
    fn lookup_finds_the_containing_file() {
        let map = map();
        let first = add(&map, "a", "abc");
        let second = add(&map, "b", "de");
        let second_start = map.file(second).expect("file exists").start();
        assert_eq!(map.lookup(second_start).map(|(id, _)| id), Some(second));
        // The end position of a file belongs to that file, so end-of-file spans
        // remain resolvable; positions past the last file do not.
        let first_end = map.file(first).expect("file exists").end();
        assert_eq!(map.lookup(first_end).map(|(id, _)| id), Some(first));
        let last_end = map.file(second).expect("file exists").end();
        assert!(
            map.lookup(BytePos::from_u32(last_end.to_u32() + 1))
                .is_none(),
            "positions past the last file are unregistered"
        );
    }

    #[test]
    fn empty_file_has_a_valid_empty_span() {
        let map = map();
        let id = add(&map, "empty", "");
        let span = map.empty_span(id).expect("file exists");
        assert!(span.is_empty());
        let snippet = map.snippet(span).expect("empty snippet exists");
        assert_eq!(snippet.text(), "");
        assert_eq!(map.file_of(span), Some(id));
    }

    #[test]
    fn large_files_are_rejected_with_a_typed_error() {
        let map = SourceMap::with_max_file_size(4);
        let error = map
            .add_file(FileName::virtual_name("huge"), "12345".to_owned())
            .expect_err("oversized file is rejected");
        assert_eq!(error, SourceMapError::FileTooLarge { len: 5, max: 4 });
        assert!(map.is_empty());
    }

    #[test]
    fn file_of_rejects_unregistered_positions() {
        let map = map();
        let id = add(&map, "a", "abc");
        let span = map.span(id, 0..1).expect("valid span");
        assert_eq!(map.file_of(span), Some(id));
        let unregistered = crate::Span::new(BytePos::from_u32(1000), BytePos::from_u32(1001));
        assert_eq!(map.file_of(unregistered), None);
    }
}
