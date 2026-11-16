//! A test fixture that owns a source map, diagnostics, and an interner.

use std::io;
use std::sync::{Arc, Mutex};

use seal_diagnostics::{DiagCtxt, Diagnostic, Emitter};
use seal_span::{FileName, SourceFile, SourceMap, Span};
use seal_symbol::{Interner, Symbol};

use crate::{Lexed, TokenKind, lex};

#[derive(Clone, Default)]
pub(crate) struct Recording(Arc<Mutex<Vec<Diagnostic>>>);

impl Emitter for Recording {
    fn emit(&mut self, diagnostic: &Diagnostic, _source_map: &SourceMap) -> io::Result<()> {
        self.0
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .push(diagnostic.clone());
        Ok(())
    }
}

impl Recording {
    fn diagnostics(&self) -> Vec<Diagnostic> {
        self.0
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone()
    }
}

pub(crate) struct Fixture {
    #[allow(dead_code)]
    pub(crate) map: Arc<SourceMap>,
    pub(crate) dcx: DiagCtxt,
    pub(crate) interner: Interner,
    pub(crate) file: Arc<SourceFile>,
    recording: Recording,
}

impl Fixture {
    pub(crate) fn new(source: &str) -> Self {
        let map = Arc::new(SourceMap::new());
        let id = map
            .add_file(FileName::virtual_name("test.sl"), source.to_owned())
            .expect("test source registers");
        let file = map.file(id).expect("registered file exists");
        let recording = Recording::default();
        let dcx = DiagCtxt::new(Box::new(recording.clone()), Arc::clone(&map));
        Self {
            map,
            dcx,
            interner: Interner::new(),
            file,
            recording,
        }
    }

    pub(crate) fn lex(&self) -> Lexed {
        lex(&self.file, &self.dcx, &self.interner)
    }

    pub(crate) fn diagnostics(&self) -> Vec<Diagnostic> {
        self.recording.diagnostics()
    }

    pub(crate) fn codes(&self) -> Vec<String> {
        self.diagnostics()
            .iter()
            .map(|diagnostic| {
                diagnostic
                    .code()
                    .map_or_else(|| "<none>".to_owned(), |code| code.as_str().to_owned())
            })
            .collect()
    }

    pub(crate) fn assert_clean(&self) {
        let diagnostics = self.diagnostics();
        assert!(
            diagnostics.is_empty(),
            "unexpected diagnostics: {:?}",
            diagnostics
                .iter()
                .map(|diagnostic| (diagnostic.code(), diagnostic.message()))
                .collect::<Vec<_>>()
        );
    }

    pub(crate) fn expect_single(&self, code: &str, message_fragment: &str) {
        let diagnostics = self.diagnostics();
        assert_eq!(
            diagnostics.len(),
            1,
            "expected one diagnostic for code {code}, got {:?}",
            diagnostics
                .iter()
                .map(|diagnostic| (diagnostic.code(), diagnostic.message()))
                .collect::<Vec<_>>()
        );
        assert_eq!(diagnostics[0].code().map(|code| code.as_str()), Some(code));
        assert!(
            diagnostics[0].message().contains(message_fragment),
            "message {:?} does not contain {:?}",
            diagnostics[0].message(),
            message_fragment
        );
    }

    pub(crate) fn text(&self, span: Span) -> &str {
        self.file.snippet(span).expect("span covers this file")
    }

    pub(crate) fn symbol(&self, symbol: Symbol) -> String {
        self.interner.resolve(symbol).to_owned()
    }

    /// Returns the source text of every token except `Eof`.
    pub(crate) fn token_texts(&self, lexed: &Lexed) -> Vec<String> {
        lexed
            .tokens
            .iter()
            .filter(|token| token.kind != TokenKind::Eof)
            .map(|token| self.text(token.span).to_owned())
            .collect()
    }

    pub(crate) fn kinds(&self, lexed: &Lexed) -> Vec<TokenKind> {
        lexed.tokens.iter().map(|token| token.kind).collect()
    }
}

/// Checks the structural invariants every lexer result must satisfy.
pub(crate) fn assert_invariants(fixture: &Fixture, lexed: &Lexed) {
    let file = &fixture.file;
    let last = lexed.tokens.last().expect("token stream is not empty");
    assert_eq!(last.kind, TokenKind::Eof, "last token must be Eof");
    assert!(last.span.is_empty(), "Eof span must be empty");
    assert_eq!(
        last.span.hi(),
        file.end(),
        "Eof must sit at the end of the file"
    );

    let mut previous_end = file.start();
    for token in &lexed.tokens {
        assert!(
            token.span.lo() >= previous_end,
            "token spans overlap: {:?} after {:?}",
            token.span,
            previous_end
        );
        assert!(
            file.contains_span(token.span),
            "token span {:?} leaves the file",
            token.span
        );
        previous_end = token.span.hi();
    }

    let mut previous_end = file.start();
    for trivia in &lexed.trivia {
        assert!(
            trivia.span.lo() >= previous_end,
            "trivia spans overlap: {:?} after {:?}",
            trivia.span,
            previous_end
        );
        assert!(
            file.contains_span(trivia.span),
            "trivia span {:?} leaves the file",
            trivia.span
        );
        previous_end = trivia.span.hi();
    }
}
