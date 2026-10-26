//! Error-handling utilities.

use std::{
    cell::Cell,
    error::Error,
    fmt::{self, Display, Formatter},
    range::Range,
    result,
};

use irs::ModuleId;

/// The [`Result`][result::Result] type alias used throughout most of the compiler. Defaults to [`HandledError`] for it's `Err` variant.
pub type Result<T, E = HandledError> = result::Result<T, E>;

/// Indicates that an error occurred, and detailed information was provided to an [`ErrorHandler`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct HandledError;

impl Display for HandledError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        "Detailed error was printed to stderr".fmt(f)
    }
}

impl Error for HandledError {}

/// A trait for user-reportable diagnostics, including errors and warnings.
pub trait Diagnostic {
    /// Any context needed to construct the report.
    type Ctx<'a>;

    /// Constructs a report from this diagnostic.
    fn report(self, ctx: Self::Ctx<'_>) -> Report;
}

/// A textual report generated from a diagnostic.
/// See [`Diagnostic`].
#[derive(Debug)]
pub struct Report {
    /// The name of the diagnostic.
    pub name: String,
    /// Whether this report is for a warning or an error.
    pub kind: ReportKind,
    /// An optional label to be attached below the highlighted source span.
    pub label: Option<String>,
    /// A list of notes to be displayed after the main report.
    pub notes: Vec<String>,
}

impl Report {
    /// Constructs a new error [`Report`].
    pub fn error(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            kind: ReportKind::Error,
            label: None,
            notes: Vec::new(),
        }
    }

    /// Constructs a new warning [`Report`].
    pub fn warning(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            kind: ReportKind::Warning,
            label: None,
            notes: Vec::new(),
        }
    }

    /// Sets the label of the [`Report`].
    #[must_use]
    pub fn with_label(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
        self
    }

    /// Attaches a note to the [`Report`].
    #[must_use]
    pub fn with_note(mut self, note: impl Into<String>) -> Self {
        self.notes.push(note.into());
        self
    }
}

impl Diagnostic for Report {
    type Ctx<'a> = ();

    fn report(self, (): ()) -> Report {
        self
    }
}

/// The type of the callback functions wrapped by [`ErrorHandler`].
pub type HandlerCallback<'cb> = &'cb dyn Fn(Report, Range<u32>, ModuleId);

/// A utility type to handle configurable, recoverable diagnostic reporting.
///
/// The primary usecase is reporting errors via [`err()`][Self::err()] and returning the [`HandledError`].
/// This type internally tracks if any errors have been reported, which can be used through [`checked()`][`Self::checked()`].
///
/// Cloning this type is cheap, and it could implement `Copy` but doesn't for similar reasons to iterators.
#[derive(Clone)]
pub struct ErrorHandler<'cb> {
    f: HandlerCallback<'cb>,
    has_err: Cell<bool>,
}

impl<'callback> ErrorHandler<'callback> {
    /// Constructs a new `ErrorHandler` with the provided reporting callback.
    pub const fn new(f: HandlerCallback<'callback>) -> Self {
        Self {
            f,
            has_err: Cell::new(false),
        }
    }

    /// Constructs an error handler for use in tests, providing simple debug output of errors.
    #[expect(clippy::use_debug, reason = "debug output is desirable for tests")]
    pub const fn test() -> Self {
        Self::new(&|report, span, module| {
            eprintln!("{report:?} (mod: {module:?}, span: {span:?})");
        })
    }

    /// Reports the provided diagnostic and produces a [`HandledError`] for the caller to use.
    pub fn report_ctx<D: Diagnostic>(
        &self,
        diagnostic: D,
        ctx: D::Ctx<'_>,
        span: impl Into<Range<u32>>,
        module: ModuleId,
    ) -> HandledError {
        let report = diagnostic.report(ctx);
        if report.kind == ReportKind::Error {
            self.has_err.set(true);
        }
        (self.f)(report, span.into(), module);
        HandledError
    }

    /// Reports the provided diagnostic and produces a [`HandledError`] for the caller to use.
    pub fn report<D>(
        &self,
        diagnostic: D,
        span: impl Into<Range<u32>>,
        module: ModuleId,
    ) -> HandledError
    where
        D: for<'a> Diagnostic<Ctx<'a> = ()>,
    {
        self.report_ctx(diagnostic, (), span, module)
    }

    /// Returns the provided value wrapped in [`Ok`], or a [`HandledError`] if this handler has reported any errors.
    ///
    /// # Errors
    /// [`HandledError`] if this handler has reported any errors.
    pub fn checked<T>(&self, val: T) -> Result<T> {
        if self.has_err.get() {
            Err(HandledError)
        } else {
            Ok(val)
        }
    }
}

/// Signals the kind of diagnostic being reported.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReportKind {
    /// A fatal error.
    Error,
    /// A warning. Compilation can continue as normal, but the programmer should be notified of something.
    Warning,
}

/// Iterator extension trait providing a method that behehaves similarly to [`try_collect`][`Iterator::try_collect`],
/// but collects all elements eagerly before returning any error.
pub trait TryCollectEager<T, E> {
    /// Similar to [`try_collect`][`Iterator::try_collect`],
    /// but eagerly collects all elements of the iterator before returning an error if any of the elements were errors.
    /// Only for unit error types.
    ///
    /// # Errors
    /// Returns the error value if any of the elements of the iterator were an error, but only after evaluating every element.
    fn try_collect_eager<U: FromIterator<T>>(self) -> Result<U, E>;
}

impl<T, I: Iterator<Item = Result<T>>> TryCollectEager<T, HandledError> for I {
    fn try_collect_eager<U: FromIterator<T>>(self) -> Result<U, HandledError> {
        try_collect_eager_helper(self, HandledError)
    }
}

impl<T, I: Iterator<Item = Result<T, ()>>> TryCollectEager<T, ()> for I {
    fn try_collect_eager<U: FromIterator<T>>(self) -> Result<U, ()> {
        try_collect_eager_helper(self, ())
    }
}

fn try_collect_eager_helper<T, U: FromIterator<T>, E>(
    iter: impl Iterator<Item = Result<T, E>>,
    err: E,
) -> Result<U, E> {
    let mut has_err = false;
    let out = iter
        .flat_map(|v| v.inspect_err(|_| has_err = true))
        .collect();
    if has_err { Err(err) } else { Ok(out) }
}
