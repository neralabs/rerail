//! Blade (`.blade.php`): a template whose PHP Rerail reads as PHP.
//!
//! A Blade tape holds the template's Blade tree and, as the root `document`'s last child, the
//! `program` its PHP makes (`view`): every echo, directive argument, `@php` block and bound
//! component attribute, in the control flow Blade compiles the template to, each node spanning the
//! template bytes it came from. Blade rules name Blade's kinds and PHP's alike
//! (`GrammarDialect::kind_id`); a PHP node is only ever inside the `program`. Value, arguments and
//! scope read the `program` as PHP 8.5 reads the compiled template: its top level is the scope of
//! the view's data, which Blade defines outside the template, so a variable the template does not
//! define is unknown there, never undefined.

pub(crate) mod forte;
#[cfg(test)]
mod oracle_tests;
#[cfg(test)]
mod tests;
pub(crate) mod view;

/// What a marker (`view`'s empty statement over a directive) does to the template's variables.
#[derive(Debug, Default, PartialEq)]
pub(crate) struct Marker {
    /// Blade assigns the variable here.
    pub(crate) define: Option<String>,
    /// Blade unsets the variable or restores what it held before: what it holds after is unknown.
    pub(crate) forget: Option<&'static str>,
    /// Blade may define any variable here (`$$name = ...`).
    pub(crate) dynamic: bool,
    /// Blade reads every variable here (`get_defined_vars()`).
    pub(crate) reads_all: bool,
}

/// The marker a directive's text makes, as `illuminate/view` 13.34.0 compiles the directive.
/// `text` is the marker's span: the directive, and for `@inject` up to the end of the variable's
/// name. `None` for text no marker spans (any PHP `;`).
pub(crate) fn marker(text: &str) -> Option<Marker> {
    let (directive, rest) = text
        .strip_prefix('@')
        .map(|rest| rest.split_at(rest.find('(').unwrap_or(rest.len())))?;
    let marker = match directive {
        "foreach" | "forelse" | "endforeach" | "empty" | "endforelse" => Marker {
            define: Some("$loop".to_owned()),
            ..Marker::default()
        },
        "error" => Marker {
            define: Some("$message".to_owned()),
            ..Marker::default()
        },
        "enderror" => Marker {
            forget: Some("$message"),
            ..Marker::default()
        },
        "session" | "context" => Marker {
            define: Some("$value".to_owned()),
            ..Marker::default()
        },
        "endsession" | "endcontext" => Marker {
            forget: Some("$value"),
            ..Marker::default()
        },
        "inject" => {
            // `@inject('name', 'Service')` is `$name = app('Service');`.
            let name = rest
                .trim_start_matches('(')
                .trim_start_matches([' ', '\t', '\'', '"']);
            if name.is_empty() {
                return None;
            }
            Marker {
                define: Some(format!("${name}")),
                ..Marker::default()
            }
        }
        "props" | "aware" => Marker {
            define: Some("$attributes".to_owned()),
            dynamic: true,
            reads_all: true,
            ..Marker::default()
        },
        "include" | "includeIf" | "includeWhen" | "includeUnless" | "includeFirst" | "each"
        | "extends" | "extendsFirst" | "component" => Marker {
            reads_all: true,
            ..Marker::default()
        },
        // An application's directive: what it compiles to may read or assign any variable.
        _ if !view::compiles(directive) => Marker {
            dynamic: true,
            reads_all: true,
            ..Marker::default()
        },
        _ => return None,
    };
    Some(marker)
}
