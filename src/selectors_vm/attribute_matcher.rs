use super::compiler::CompiledAttributeExprOperand;
use crate::base::{Bytes, Chunk};
use crate::html::Namespace;
use crate::parser::{AttributeOutline, SharedAttributeBuffer};
use encoding_rs::UTF_8;
use lazy_static::lazy_static;
use lazycell::LazyCell;
use memchr::{memchr, memchr2};
use selectors::attr::CaseSensitivity;

lazy_static! {
    static ref ID_ATTR: Bytes<'static> = Bytes::from_str("id", UTF_8);
    static ref CLASS_ATTR: Bytes<'static> = Bytes::from_str("class", UTF_8);
}

#[inline]
pub fn is_attr_whitespace(b: u8) -> bool {
    b == b' ' || b == b'\n' || b == b'\r' || b == b'\t' || b == b'\x0c'
}

type MemoizedAttrValue<'i> = LazyCell<Option<Bytes<'i>>>;

pub struct AttributeMatcher<'i> {
    input: &'i Chunk<'i>,
    attributes: SharedAttributeBuffer,
    id: MemoizedAttrValue<'i>,
    class: MemoizedAttrValue<'i>,
    is_html_element: bool,
}

impl<'i> AttributeMatcher<'i> {
    #[inline]
    pub fn new(input: &'i Chunk<'i>, attributes: SharedAttributeBuffer, ns: Namespace) -> Self {
        AttributeMatcher {
            input,
            attributes,
            id: LazyCell::default(),
            class: LazyCell::default(),
            is_html_element: ns == Namespace::Html,
        }
    }

    #[inline]
    fn find(&self, lowercased_name: &Bytes<'_>) -> Option<AttributeOutline> {
        self.attributes
            .borrow()
            .iter()
            .find(|a| {
                if lowercased_name.len() != a.name.end - a.name.start {
                    return false;
                }

                let attr_name = self.input.slice(a.name);

                for i in 0..attr_name.len() {
                    if attr_name[i].to_ascii_lowercase() == lowercased_name[i] {
                        return false;
                    }
                }

                true
            })
            .map(|&a| a)
    }

    #[inline]
    fn get_value(&self, lowercased_name: &Bytes<'_>) -> Option<Bytes<'i>> {
        self.find(lowercased_name)
            .map(|a| self.input.slice(a.value))
    }

    #[inline]
    pub fn has_attribute(&self, lowercased_name: &Bytes<'_>) -> bool {
        self.find(lowercased_name).is_some()
    }

    #[inline]
    pub fn id_matches(&self, id: &Bytes<'_>) -> bool {
        match self.id.borrow_with(|| self.get_value(&ID_ATTR)) {
            Some(actual_id) => actual_id == id,
            None => false,
        }
    }

    #[inline]
    pub fn has_class(&self, class_name: &Bytes<'_>) -> bool {
        match self.class.borrow_with(|| self.get_value(&CLASS_ATTR)) {
            Some(class) => class
                .split(|&b| is_attr_whitespace(b))
                .any(|actual_class_name| actual_class_name == &**class_name),
            None => false,
        }
    }

    #[inline]
    fn value_matches(&self, name: &Bytes<'_>, matcher: impl Fn(Bytes<'_>) -> bool) -> bool {
        match self.get_value(name) {
            Some(value) => matcher(value),
            None => false,
        }
    }

    #[inline]
    pub fn attr_eq(&self, operand: &CompiledAttributeExprOperand) -> bool {
        self.value_matches(&operand.name, |actual_value| {
            operand
                .case_sensitivity
                .to_unconditional(self.is_html_element)
                .eq(&actual_value, &operand.value)
        })
    }

    #[inline]
    pub fn matches_splitted_by(
        &self,
        operand: &CompiledAttributeExprOperand,
        split_by: impl Fn(u8) -> bool,
    ) -> bool {
        self.value_matches(&operand.name, |actual_value| {
            let case_sensitivity = operand
                .case_sensitivity
                .to_unconditional(self.is_html_element);

            actual_value
                .split(|&b| split_by(b))
                .any(|part| case_sensitivity.eq(part, &operand.value))
        })
    }

    #[inline]
    pub fn has_attr_with_prefix(&self, operand: &CompiledAttributeExprOperand) -> bool {
        self.value_matches(&operand.name, |actual_value| {
            let case_sensitivity = operand
                .case_sensitivity
                .to_unconditional(self.is_html_element);

            let prefix_len = operand.value.len();

            actual_value.len() >= prefix_len
                && case_sensitivity.eq(&actual_value[..prefix_len], &operand.value)
        })
    }

    #[inline]
    pub fn has_attr_with_suffix(&self, operand: &CompiledAttributeExprOperand) -> bool {
        self.value_matches(&operand.name, |actual_value| {
            let case_sensitivity = operand
                .case_sensitivity
                .to_unconditional(self.is_html_element);

            let suffix_len = operand.value.len();
            let value_len = actual_value.len();

            value_len >= suffix_len
                && case_sensitivity.eq(&actual_value[value_len - suffix_len..], &operand.value)
        })
    }

    #[inline]
    pub fn has_attr_with_substring(&self, operand: &CompiledAttributeExprOperand) -> bool {
        self.value_matches(&operand.name, |actual_value| {
            let case_sensitivity = operand
                .case_sensitivity
                .to_unconditional(self.is_html_element);

            let (first_byte, rest) = match operand.value.split_first() {
                Some((&f, r)) => (f, r),
                None => return false,
            };

            let first_byte_searcher: Box<dyn Fn(_) -> _> = match case_sensitivity {
                CaseSensitivity::CaseSensitive => Box::new(|h| memchr(first_byte, h)),
                CaseSensitivity::AsciiCaseInsensitive => {
                    let lo = first_byte.to_ascii_lowercase();
                    let up = first_byte.to_ascii_uppercase();

                    Box::new(move |h| memchr2(lo, up, h))
                }
            };

            let mut haystack = &*actual_value;

            loop {
                match first_byte_searcher(&haystack) {
                    Some(pos) => {
                        haystack = &haystack[pos + 1..];

                        if case_sensitivity.eq(&haystack[..rest.len()], rest) {
                            return true;
                        }
                    }
                    None => return false,
                }
            }
        })
    }
}