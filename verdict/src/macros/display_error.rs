/// Define an error enum with `Display` and `Error` impls.
///
/// Does not generate an `Actionable` impl, making it suitable for crates
/// that depend on verdict only optionally.
///
/// Each variant requires a `#[display("...")]` string with optional field
/// interpolation.
///
/// # Example
///
/// ```
/// use verdict::display_error;
///
/// display_error! {
///     #[derive(Clone, PartialEq, Eq)]
///     pub enum StorageError {
///         #[display("I/O error")]
///         Io,
///
///         #[display("not found")]
///         NotFound,
///
///         #[display("checksum mismatch: expected {expected:#x}, got {actual:#x}")]
///         ChecksumMismatch { expected: u32, actual: u32 },
///     }
/// }
///
/// let err = StorageError::Io;
/// assert_eq!(err.to_string(), "I/O error");
///
/// let err = StorageError::ChecksumMismatch { expected: 0xAB, actual: 0xCD };
/// assert_eq!(err.to_string(), "checksum mismatch: expected 0xab, got 0xcd");
/// ```
///
/// # Generic Example
///
/// ```
/// use verdict::display_error;
///
/// display_error! {
///     pub enum MyError<Id: core::fmt::Debug, E: core::fmt::Debug + core::fmt::Display> {
///         #[display("not found: {id:?}")]
///         NotFound { id: Id },
///
///         #[display("inner error: {source}")]
///         Inner { source: E },
///     }
/// }
///
/// let err = MyError::<u32, String>::NotFound { id: 42 };
/// assert_eq!(err.to_string(), "not found: 42");
/// ```
#[macro_export]
macro_rules! display_error {
    // ── Entry point: GENERIC — detect `<` after enum name ──────────────
    (
        $(#[$attr:meta])*
        $vis:vis enum $name:ident < $($rest:tt)+
    ) => {
        $crate::display_error!(
            @collect_generics
            { $(#[$attr])* }
            $vis
            $name
            []
            $($rest)+
        );
    };

    // ── Entry point: NON-GENERIC (original) ────────────────────────────
    (
        $(#[$attr:meta])*
        $vis:vis enum $name:ident {
            $(
                #[display($fmt:literal)]
                $variant:ident $({ $($field:ident : $fty:ty),* $(,)? })?
            ),* $(,)?
        }
    ) => {
        #[derive(Debug)]
        $(#[$attr])*
        #[must_use]
        $vis enum $name {
            $(
                $variant $({ $($field : $fty),* })?,
            )*
        }

        impl ::core::error::Error for $name {}

        $crate::display_error!(@build_display $name [] $(
            [$variant $({ $($field),* })? => $fmt]
        )*);
    };

    // ── @collect_generics: munch until `> {` ───────────────────────────

    // Found `> { ... }` — generics complete
    (@collect_generics
        { $($attrs:tt)* }
        $vis:vis
        $name:ident
        [ $($gen:tt)* ]
        > { $($body:tt)* }
    ) => {
        $crate::display_error!(
            @strip_bounds
            { $($attrs)* }
            $vis
            $name
            [ $($gen)* ]       // bounded generics (original)
            { $($body)* }      // variant body
            []                 // bare ident accumulator
            [ $($gen)* ]       // remaining tokens to walk
        );
    };

    // Consume one token
    (@collect_generics
        { $($attrs:tt)* }
        $vis:vis
        $name:ident
        [ $($gen:tt)* ]
        $tok:tt $($rest:tt)*
    ) => {
        $crate::display_error!(
            @collect_generics
            { $($attrs)* }
            $vis
            $name
            [ $($gen)* $tok ]
            $($rest)*
        );
    };

    // ── @strip_bounds: extract bare param idents from generics ─────────

    // Done: all tokens consumed
    (@strip_bounds
        { $($attrs:tt)* }
        $vis:vis
        $name:ident
        [ $($bounded:tt)* ]
        { $($body:tt)* }
        [ $($bare:ident),* ]
        []
    ) => {
        $crate::display_error!(
            @define_generic
            { $($attrs)* }
            $vis
            $name
            [ $($bare),* ]
            [ $($bounded)* ]
            { $($body)* }
        );
    };

    // Param with colon — has bounds
    (@strip_bounds
        { $($attrs:tt)* }
        $vis:vis
        $name:ident
        [ $($bounded:tt)* ]
        { $($body:tt)* }
        [ $($bare:ident),* ]
        [ $param:ident : $($rest:tt)* ]
    ) => {
        $crate::display_error!(
            @skip_bounds
            { $($attrs)* }
            $vis
            $name
            [ $($bounded)* ]
            { $($body)* }
            [ $($bare,)* $param ]
            [ $($rest)* ]
        );
    };

    // Param followed by comma — no bounds
    (@strip_bounds
        { $($attrs:tt)* }
        $vis:vis
        $name:ident
        [ $($bounded:tt)* ]
        { $($body:tt)* }
        [ $($bare:ident),* ]
        [ $param:ident , $($rest:tt)* ]
    ) => {
        $crate::display_error!(
            @strip_bounds
            { $($attrs)* }
            $vis
            $name
            [ $($bounded)* ]
            { $($body)* }
            [ $($bare,)* $param ]
            [ $($rest)* ]
        );
    };

    // Last param — no bounds, no trailing comma
    (@strip_bounds
        { $($attrs:tt)* }
        $vis:vis
        $name:ident
        [ $($bounded:tt)* ]
        { $($body:tt)* }
        [ $($bare:ident),* ]
        [ $param:ident ]
    ) => {
        $crate::display_error!(
            @strip_bounds
            { $($attrs)* }
            $vis
            $name
            [ $($bounded)* ]
            { $($body)* }
            [ $($bare,)* $param ]
            []
        );
    };

    // ── @skip_bounds: consume bound tokens until comma or end ──────────

    // Found comma — bounds done, continue stripping
    (@skip_bounds
        { $($attrs:tt)* }
        $vis:vis
        $name:ident
        [ $($bounded:tt)* ]
        { $($body:tt)* }
        [ $($bare:ident),* ]
        [ , $($rest:tt)* ]
    ) => {
        $crate::display_error!(
            @strip_bounds
            { $($attrs)* }
            $vis
            $name
            [ $($bounded)* ]
            { $($body)* }
            [ $($bare),* ]
            [ $($rest)* ]
        );
    };

    // Empty — last param's bounds consumed
    (@skip_bounds
        { $($attrs:tt)* }
        $vis:vis
        $name:ident
        [ $($bounded:tt)* ]
        { $($body:tt)* }
        [ $($bare:ident),* ]
        []
    ) => {
        $crate::display_error!(
            @strip_bounds
            { $($attrs)* }
            $vis
            $name
            [ $($bounded)* ]
            { $($body)* }
            [ $($bare),* ]
            []
        );
    };

    // Consume one bound token
    (@skip_bounds
        { $($attrs:tt)* }
        $vis:vis
        $name:ident
        [ $($bounded:tt)* ]
        { $($body:tt)* }
        [ $($bare:ident),* ]
        [ $tok:tt $($rest:tt)* ]
    ) => {
        $crate::display_error!(
            @skip_bounds
            { $($attrs)* }
            $vis
            $name
            [ $($bounded)* ]
            { $($body)* }
            [ $($bare),* ]
            [ $($rest)* ]
        );
    };

    // ── @define_generic: generate enum + Error + Display ───────────────
    (@define_generic
        { $($attrs:tt)* }
        $vis:vis
        $name:ident
        [ $($param:ident),+ ]
        [ $($bounded:tt)+ ]
        {
            $(
                #[display($fmt:literal)]
                $variant:ident $({ $($field:ident : $fty:ty),* $(,)? })?
            ),* $(,)?
        }
    ) => {
        #[derive(Debug)]
        $($attrs)*
        #[must_use]
        $vis enum $name < $($param),+ > {
            $(
                $variant $({ $($field : $fty),* })?,
            )*
        }

        impl< $($bounded)+ > ::core::error::Error for $name < $($param),+ > {}

        $crate::display_error!(
            @build_display_generic
            $name
            [ $($param),+ ]
            [ $($bounded)+ ]
            []
            $(
                [$variant $({ $($field),* })? => $fmt]
            )*
        );
    };

    // ── Display TT-muncher (generic) ───────────────────────────────────

    // Base case
    (@build_display_generic
        $name:ident
        [ $($param:ident),+ ]
        [ $($bounded:tt)+ ]
        [ $($arms:tt)* ]
    ) => {
        $crate::display_error!(
            @emit_display_generic
            $name
            [ $($param),+ ]
            [ $($bounded)+ ]
            $($arms)*
        );
    };

    // Struct variant
    (@build_display_generic
        $name:ident
        [ $($param:ident),+ ]
        [ $($bounded:tt)+ ]
        [ $($arms:tt)* ]
        [$variant:ident { $($field:ident),* } => $fmt:literal]
        $($rest:tt)*
    ) => {
        $crate::display_error!(
            @build_display_generic
            $name
            [ $($param),+ ]
            [ $($bounded)+ ]
            [
                $($arms)*
                { $name :: $variant { $($field),* } => $fmt }
            ]
            $($rest)*
        );
    };

    // Unit variant
    (@build_display_generic
        $name:ident
        [ $($param:ident),+ ]
        [ $($bounded:tt)+ ]
        [ $($arms:tt)* ]
        [$variant:ident => $fmt:literal]
        $($rest:tt)*
    ) => {
        $crate::display_error!(
            @build_display_generic
            $name
            [ $($param),+ ]
            [ $($bounded)+ ]
            [
                $($arms)*
                { $name :: $variant => $fmt }
            ]
            $($rest)*
        );
    };

    // ── Emit Display impl (generic) ────────────────────────────────────
    (@emit_display_generic
        $name:ident
        [ $($param:ident),+ ]
        [ $($bounded:tt)+ ]
        $( { $pat:pat => $fmt:literal } )*
    ) => {
        impl< $($bounded)+ > ::core::fmt::Display for $name < $($param),+ > {
            #[allow(unused_variables)]
            fn fmt(&self, f: &mut ::core::fmt::Formatter<'_>) -> ::core::fmt::Result {
                match self {
                    $(
                        $pat => ::core::write!(f, $fmt),
                    )*
                }
            }
        }
    };

    // ── Display TT-muncher (non-generic, original) ─────────────────────

    (@build_display $name:ident [ $($arms:tt)* ]) => {
        $crate::display_error!(@emit_display $name $($arms)*);
    };

    (@build_display $name:ident [ $($arms:tt)* ]
        [$variant:ident { $($field:ident),* } => $fmt:literal]
        $($rest:tt)*
    ) => {
        $crate::display_error!(@build_display $name [
            $($arms)*
            { $name :: $variant { $($field),* } => $fmt }
        ] $($rest)*);
    };

    (@build_display $name:ident [ $($arms:tt)* ]
        [$variant:ident => $fmt:literal]
        $($rest:tt)*
    ) => {
        $crate::display_error!(@build_display $name [
            $($arms)*
            { $name :: $variant => $fmt }
        ] $($rest)*);
    };

    // ── Emit Display impl (non-generic, original) ──────────────────────
    (@emit_display $name:ident $( { $pat:pat => $fmt:literal } )* ) => {
        impl ::core::fmt::Display for $name {
            #[allow(unused_variables)]
            fn fmt(&self, f: &mut ::core::fmt::Formatter<'_>) -> ::core::fmt::Result {
                match self {
                    $(
                        $pat => ::core::write!(f, $fmt),
                    )*
                }
            }
        }
    };
}
