//! Small attribute macro for opt-in function invocation profiling.

use proc_macro::{Delimiter, Group, TokenStream, TokenTree};

/// Records an invocation when the `profiling` feature is enabled.
#[proc_macro_attribute]
pub fn profiled(_attribute: TokenStream, item: TokenStream) -> TokenStream {
    let tokens: Vec<_> = item.into_iter().collect();
    let mut function_name = None;
    for pair in tokens.windows(2) {
        if matches!(&pair[0], TokenTree::Ident(ident) if ident.to_string() == "fn") {
            if let TokenTree::Ident(name) = &pair[1] {
                function_name = Some(name.to_string());
                break;
            }
        }
    }
    let Some(function_name) = function_name else {
        return "compile_error!(\"#[profiled] can only be used on a function\");"
            .parse()
            .expect("valid compile_error token stream");
    };

    let Some(body_index) = tokens.iter().rposition(
        |token| matches!(token, TokenTree::Group(group) if group.delimiter() == Delimiter::Brace),
    ) else {
        return "compile_error!(\"#[profiled] function must have a body\");"
            .parse()
            .expect("valid compile_error token stream");
    };

    let hook: TokenStream = format!(
        "#[cfg(feature = \"profiling\")] crate::profiling::record_function(concat!(module_path!(), \"::\", \"{function_name}\"));"
    )
    .parse()
    .expect("generated profiling hook is valid Rust");
    let mut body = hook;
    if let TokenTree::Group(original) = &tokens[body_index] {
        body.extend(original.stream());
        let mut replacement = Group::new(Delimiter::Brace, body);
        replacement.set_span(original.span());
        let mut output = TokenStream::new();
        for (index, token) in tokens.into_iter().enumerate() {
            output.extend([if index == body_index {
                TokenTree::Group(replacement.clone())
            } else {
                token
            }]);
        }
        output
    } else {
        unreachable!("body index points to a group")
    }
}
