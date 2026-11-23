//! Forte's `Parser\OptionalTags\OptionalTagConfig` and `OptionalTagHandler`: the elements whose
//! closing tags HTML lets a template omit, and when the tree builder closes them for it.

/// A rule of `CLOSING_TAG_OMISSION_RULES`: `auto_close_when_sibling` and
/// `auto_close_at_parent_end`, each absent or a list.
struct Rule {
    when_sibling: Option<&'static [&'static [u8]]>,
    at_parent_end: Option<&'static [&'static [u8]]>,
}

/// `getClosingConditions`: the element's rule, if it can omit its closing tag.
fn get_closing_conditions(element_name: &[u8]) -> Option<Rule> {
    const LI: Rule = Rule {
        when_sibling: Some(&[b"li"]),
        at_parent_end: Some(&[b"ul", b"ol", b"menu"]),
    };
    const DT_DD: Rule = Rule {
        when_sibling: Some(&[b"dt", b"dd"]),
        at_parent_end: Some(&[b"dl"]),
    };
    const P: Rule = Rule {
        when_sibling: Some(&[
            b"address",
            b"article",
            b"aside",
            b"blockquote",
            b"div",
            b"dl",
            b"fieldset",
            b"footer",
            b"form",
            b"h1",
            b"h2",
            b"h3",
            b"h4",
            b"h5",
            b"h6",
            b"header",
            b"hr",
            b"main",
            b"nav",
            b"ol",
            b"p",
            b"pre",
            b"section",
            b"table",
            b"ul",
        ]),
        at_parent_end: None,
    };
    const OPTION: Rule = Rule {
        when_sibling: Some(&[b"option", b"optgroup"]),
        at_parent_end: Some(&[b"select", b"datalist", b"optgroup"]),
    };
    const OPTGROUP: Rule = Rule {
        when_sibling: Some(&[b"optgroup"]),
        at_parent_end: Some(&[b"select"]),
    };
    const RT_RP: Rule = Rule {
        when_sibling: Some(&[b"rt", b"rp"]),
        at_parent_end: Some(&[b"ruby", b"rtc"]),
    };
    const RB: Rule = Rule {
        when_sibling: Some(&[b"rb", b"rt", b"rp", b"rtc"]),
        at_parent_end: Some(&[b"ruby"]),
    };
    const CAPTION_COLGROUP: Rule = Rule {
        when_sibling: Some(&[b"colgroup", b"thead", b"tbody", b"tfoot", b"tr"]),
        at_parent_end: Some(&[b"table"]),
    };
    const THEAD_TBODY: Rule = Rule {
        when_sibling: Some(&[b"tbody", b"tfoot"]),
        at_parent_end: Some(&[b"table"]),
    };
    const TFOOT: Rule = Rule {
        when_sibling: None,
        at_parent_end: Some(&[b"table"]),
    };
    const TR: Rule = Rule {
        when_sibling: Some(&[b"tr"]),
        at_parent_end: Some(&[b"table", b"thead", b"tbody", b"tfoot"]),
    };
    const TD_TH: Rule = Rule {
        when_sibling: Some(&[b"td", b"th"]),
        at_parent_end: Some(&[b"tr"]),
    };
    Some(match element_name {
        b"li" => LI,
        b"dt" | b"dd" => DT_DD,
        b"p" => P,
        b"option" => OPTION,
        b"optgroup" => OPTGROUP,
        b"rt" | b"rp" => RT_RP,
        b"rb" => RB,
        b"caption" | b"colgroup" => CAPTION_COLGROUP,
        b"thead" | b"tbody" => THEAD_TBODY,
        b"tfoot" => TFOOT,
        b"tr" => TR,
        b"td" | b"th" => TD_TH,
        _ => return None,
    })
}

/// `in_array($name, $list, true)` for an optional name.
fn in_list(name: Option<&[u8]>, list: &[&[u8]]) -> bool {
    name.is_some_and(|name| list.contains(&name))
}

/// `OptionalTagConfig::shouldAutoCloseOnSibling`.
fn should_auto_close_on_sibling(element: &[u8], sibling_element: &[u8]) -> bool {
    get_closing_conditions(element)
        .and_then(|rule| rule.when_sibling)
        .is_some_and(|siblings| in_list(Some(sibling_element), siblings))
}

/// `OptionalTagHandler::shouldAutoCloseElement` as the tree builder calls it: with a next
/// element and not at the parent's end.
pub(super) fn should_auto_close_element(element_name: &[u8], next_element: &[u8]) -> bool {
    // `canOmitClosingTag`, then the sibling rule; `atParentEnd` is always false here.
    get_closing_conditions(element_name).is_some()
        && should_auto_close_on_sibling(element_name, next_element)
}

/// `OptionalTagHandler::isInValidParentContext`.
pub(super) fn is_in_valid_parent_context(
    element_name: &[u8],
    parent_element: Option<&[u8]>,
) -> bool {
    let Some(conditions) = get_closing_conditions(element_name) else {
        return false;
    };
    let has_parent_rules = conditions.at_parent_end.is_some();
    let has_sibling_rules = conditions.when_sibling.is_some();
    // Guard against self-nesting and missing parent where required.
    if (parent_element == Some(element_name) || parent_element.is_none())
        && (has_parent_rules || parent_element == Some(element_name))
    {
        return false;
    }
    if let Some(valid_parents) = conditions.at_parent_end {
        return in_list(parent_element, valid_parents);
    }
    if has_sibling_rules {
        return parent_element.is_some();
    }
    false
}

/// `validParentsFor` (`ProcessesElementAutoClosing`): the rule's `auto_close_at_parent_end`.
pub(super) fn valid_parents_for(tag: &[u8]) -> Option<&'static [&'static [u8]]> {
    get_closing_conditions(tag).and_then(|rule| rule.at_parent_end)
}
