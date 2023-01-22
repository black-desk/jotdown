use crate::OrderedListNumbering::*;
use crate::OrderedListStyle::*;
use crate::Span;
use crate::EOF;

use crate::attr;
use crate::tree;

use Atom::*;
use Container::*;
use Leaf::*;
use ListType::*;

pub type Tree = tree::Tree<Node, Atom>;
pub type TreeBuilder = tree::Builder<Node, Atom>;
pub type Element = tree::Element<Node, Atom>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Node {
    Container(Container),
    Leaf(Leaf),
}

#[must_use]
pub fn parse(src: &str) -> Tree {
    TreeParser::new(src).parse()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Block {
    /// An atomic block, containing no children elements.
    Atom(Atom),

    /// A leaf block, containing only inline elements.
    Leaf(Leaf),

    /// A container block, containing children blocks.
    Container(Container),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Atom {
    /// A line with no non-whitespace characters.
    Blankline,

    /// A list of attributes.
    Attributes,

    /// A thematic break.
    ThematicBreak,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Leaf {
    /// Span is empty, before first character of paragraph.
    /// Each inline is a line.
    Paragraph,

    /// Span is `#` characters.
    /// Each inline is a line.
    Heading,

    /// Span is first `|` character.
    /// Each inline is a line (row).
    Table,

    /// Span is the link tag.
    /// Inlines are lines of the URL.
    LinkDefinition,

    /// Span is language specifier.
    /// Each inline is a line.
    CodeBlock,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Container {
    /// Span is `>`.
    Blockquote,

    /// Span is class specifier, possibly empty.
    Div,

    /// Span is the list marker of the first list item in the list.
    List(ListType),

    /// Span is the list marker.
    ListItem(ListType),

    /// Span is footnote tag.
    Footnote,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ListType {
    Unordered(u8),
    Ordered(crate::OrderedListNumbering, crate::OrderedListStyle),
    Task,
    Description,
}

#[derive(Debug)]
struct OpenList {
    /// Type of the list, used to determine whether this list should be continued or a new one
    /// should be created.
    ty: ListType,
    /// Depth in the tree where the direct list items of the list are. Needed to determine when to
    /// close the list.
    depth: u16,
}

/// Parser for block-level tree structure of entire document.
struct TreeParser<'s> {
    src: &'s str,
    tree: TreeBuilder,

    lists_open: Vec<OpenList>,
}

impl<'s> TreeParser<'s> {
    #[must_use]
    pub fn new(src: &'s str) -> Self {
        Self {
            src,
            tree: TreeBuilder::new(),
            lists_open: Vec::new(),
        }
    }

    #[must_use]
    pub fn parse(mut self) -> Tree {
        let mut lines = lines(self.src).collect::<Vec<_>>();
        let mut line_pos = 0;
        while line_pos < lines.len() {
            let line_count = self.parse_block(&mut lines[line_pos..]);
            if line_count == 0 {
                break;
            }
            line_pos += line_count;
        }
        for _ in self.lists_open.drain(..) {
            self.tree.exit(); // list
        }
        self.tree.finish()
    }

    /// Recursively parse a block and all of its children. Return number of lines the block uses.
    fn parse_block(&mut self, lines: &mut [Span]) -> usize {
        BlockParser::parse(lines.iter().map(|sp| sp.of(self.src))).map_or(
            0,
            |(indent, kind, span, line_count)| {
                let truncated = line_count > lines.len();
                let l = line_count.min(lines.len());
                let lines = &mut lines[..l];
                let span = span.translate(lines[0].start());

                // skip part of first inline that is shared with the block span
                lines[0] = lines[0].with_start(span.end());

                // remove junk from footnotes / link defs
                if matches!(
                    kind,
                    Block::Leaf(LinkDefinition) | Block::Container(Footnote { .. })
                ) {
                    assert_eq!(&lines[0].of(self.src).chars().as_str()[0..2], "]:");
                    lines[0] = lines[0].skip(2);
                }

                // skip closing fence of code blocks / divs
                let lines = if !truncated
                    && matches!(kind, Block::Leaf(CodeBlock) | Block::Container(Div))
                {
                    let l = lines.len();
                    &mut lines[..l - 1]
                } else {
                    lines
                };

                match kind {
                    Block::Atom(a) => self.tree.atom(a, span),
                    Block::Leaf(l) => {
                        self.tree.enter(Node::Leaf(l), span);

                        if matches!(l, Leaf::CodeBlock) {
                            lines[0] = lines[0].trim_start(self.src);
                        } else {
                            // trim starting whitespace of each inline
                            for line in lines.iter_mut() {
                                *line = line.trim_start(self.src);
                            }

                            // trim ending whitespace of block
                            let l = lines.len();
                            if l > 0 {
                                let last = &mut lines[l - 1];
                                *last = last.trim_end(self.src);
                            }
                        }

                        // skip first inline if empty (e.g. code block)
                        let lines = if lines[0].is_empty() {
                            &mut lines[1..]
                        } else {
                            lines
                        };

                        lines.iter().for_each(|line| self.tree.inline(*line));
                        self.tree.exit();
                    }
                    Block::Container(c) => {
                        let (skip_chars, skip_lines_suffix) = match c {
                            Blockquote => (2, 0),
                            List(..) => panic!(),
                            ListItem(..) | Footnote => (indent, 0),
                            Div => (0, 1),
                        };
                        let line_count_inner = lines.len() - skip_lines_suffix;

                        // update spans, remove indentation / container prefix
                        lines
                            .iter_mut()
                            .skip(1)
                            .take(line_count_inner)
                            .for_each(|sp| {
                                let skip = (sp
                                    .of(self.src)
                                    .chars()
                                    .take_while(|c| c.is_whitespace())
                                    .count()
                                    + skip_chars)
                                    .min(sp.len() - usize::from(sp.of(self.src).ends_with('\n')));
                                *sp = sp.skip(skip);
                            });

                        if let Container::ListItem(ty) = c {
                            if self
                                .lists_open
                                .last()
                                .map_or(true, |OpenList { depth, .. }| {
                                    usize::from(*depth) < self.tree.depth()
                                })
                            {
                                self.tree.enter(Node::Container(Container::List(ty)), span);
                                self.lists_open.push(OpenList {
                                    ty,
                                    depth: self.tree.depth().try_into().unwrap(),
                                });
                            }
                        }

                        self.tree.enter(Node::Container(c), span);
                        let mut l = 0;
                        while l < line_count_inner {
                            l += self.parse_block(&mut lines[l..line_count_inner]);
                        }

                        if let Some(OpenList { depth, .. }) = self.lists_open.last() {
                            assert!(usize::from(*depth) <= self.tree.depth());
                            if self.tree.depth() == (*depth).into() {
                                self.tree.exit(); // list
                                self.lists_open.pop();
                            }
                        }

                        self.tree.exit();
                    }
                }

                line_count
            },
        )
    }
}

/// Parser for a single block.
struct BlockParser {
    indent: usize,
    kind: Block,
    span: Span,
    fence: Option<(char, usize)>,
}

impl BlockParser {
    /// Parse a single block. Return number of lines the block uses.
    fn parse<'s, I: Iterator<Item = &'s str>>(mut lines: I) -> Option<(usize, Block, Span, usize)> {
        lines.next().map(|l| {
            let mut p = BlockParser::new(l);
            let has_end_delimiter =
                matches!(p.kind, Block::Leaf(CodeBlock) | Block::Container(Div));
            let line_count_match = lines.take_while(|l| p.continues(l)).count();
            let line_count = 1 + line_count_match + usize::from(has_end_delimiter);
            (p.indent, p.kind, p.span, line_count)
        })
    }

    fn new(line: &str) -> Self {
        let start = line
            .chars()
            .take_while(|c| *c != '\n' && c.is_whitespace())
            .count();
        let line_t = &line[start..];
        let mut chars = line_t.chars();

        let mut fence = None;
        let (kind, span) = match chars.next().unwrap_or(EOF) {
            EOF => Some((Block::Atom(Blankline), Span::empty_at(start))),
            '\n' => Some((Block::Atom(Blankline), Span::by_len(start, 1))),
            '#' => chars
                .find(|c| *c != '#')
                .map_or(true, char::is_whitespace)
                .then(|| {
                    (
                        Block::Leaf(Heading),
                        Span::by_len(start, line_t.len() - chars.as_str().len() - 1),
                    )
                }),
            '>' => {
                if let Some(c) = chars.next() {
                    c.is_whitespace().then(|| {
                        (
                            Block::Container(Blockquote),
                            Span::by_len(start, line_t.len() - chars.as_str().len() - 1),
                        )
                    })
                } else {
                    Some((
                        Block::Container(Blockquote),
                        Span::by_len(start, line_t.len() - chars.as_str().len()),
                    ))
                }
            }
            '{' => (attr::valid(line_t.chars()).0 == line_t.trim_end().len())
                .then(|| (Block::Atom(Attributes), Span::by_len(start, line_t.len()))),
            '|' => (&line_t[line_t.len() - 1..] == "|"
                && &line_t[line_t.len() - 2..line_t.len() - 1] != "\\")
                .then(|| (Block::Leaf(Table), Span::by_len(start, 1))),
            '[' => chars.as_str().find("]:").map(|l| {
                let tag = &chars.as_str()[0..l];
                let (tag, is_footnote) = if let Some(tag) = tag.strip_prefix('^') {
                    (tag, true)
                } else {
                    (tag, false)
                };
                (
                    if is_footnote {
                        Block::Container(Footnote)
                    } else {
                        Block::Leaf(LinkDefinition)
                    },
                    Span::from_slice(line, tag),
                )
            }),
            '-' | '*' if Self::is_thematic_break(chars.clone()) => Some((
                Block::Atom(ThematicBreak),
                Span::from_slice(line, line_t.trim()),
            )),
            b @ ('-' | '*' | '+') => chars.next().map_or(true, char::is_whitespace).then(|| {
                let task_list = chars.next() == Some('[')
                    && matches!(chars.next(), Some('x' | 'X' | ' '))
                    && chars.next() == Some(']')
                    && chars.next().map_or(true, char::is_whitespace);
                if task_list {
                    (Block::Container(ListItem(Task)), Span::by_len(start, 5))
                } else {
                    (
                        Block::Container(ListItem(Unordered(b as u8))),
                        Span::by_len(start, 1),
                    )
                }
            }),
            ':' if chars.clone().next().map_or(true, char::is_whitespace) => Some((
                Block::Container(ListItem(Description)),
                Span::by_len(start, 1),
            )),
            f @ ('`' | ':' | '~') => {
                let fence_length = (&mut chars).take_while(|c| *c == f).count() + 1;
                fence = Some((f, fence_length));
                let lang = line_t[fence_length..].trim();
                let valid_spec =
                    !lang.chars().any(char::is_whitespace) && !lang.chars().any(|c| c == '`');
                (valid_spec && fence_length >= 3).then(|| {
                    (
                        match f {
                            ':' => Block::Container(Div),
                            _ => Block::Leaf(CodeBlock),
                        },
                        Span::from_slice(line, lang),
                    )
                })
            }
            c => Self::maybe_ordered_list_item(c, &mut chars).map(|(num, style, len)| {
                (
                    Block::Container(ListItem(Ordered(num, style))),
                    Span::by_len(start, len),
                )
            }),
        }
        .unwrap_or((Block::Leaf(Paragraph), Span::new(0, 0)));

        Self {
            indent: start,
            kind,
            span,
            fence,
        }
    }

    fn is_thematic_break(chars: std::str::Chars) -> bool {
        let mut n = 1;
        for c in chars {
            if matches!(c, '-' | '*') {
                n += 1;
            } else if !c.is_whitespace() {
                return false;
            }
        }
        n >= 3
    }

    /// Determine if this line continues the block.
    fn continues(&mut self, line: &str) -> bool {
        let line_t = line.trim();
        let empty = line_t.is_empty();
        match self.kind {
            Block::Atom(..) => false,
            Block::Leaf(Paragraph | Heading | Table) => !line.trim().is_empty(),
            Block::Leaf(LinkDefinition) => line.starts_with(' ') && !line.trim().is_empty(),
            Block::Container(Blockquote) => line.trim().starts_with('>'),
            Block::Container(ListItem(..)) => {
                let spaces = line.chars().take_while(|c| c.is_whitespace()).count();
                empty
                    || spaces > self.indent
                    || matches!(
                        Self::parse(std::iter::once(line)),
                        Some((_, Block::Leaf(Leaf::Paragraph), _, _)),
                    )
            }
            Block::Container(Footnote) => {
                let spaces = line.chars().take_while(|c| c.is_whitespace()).count();
                empty || spaces > self.indent
            }
            Block::Container(Div) | Block::Leaf(CodeBlock) => {
                let (fence, fence_length) = self.fence.unwrap();
                let mut c = line.chars();
                !((&mut c).take(fence_length).all(|c| c == fence)
                    && c.next().map_or(true, char::is_whitespace))
            }
            Block::Container(List(..)) => panic!(),
        }
    }

    fn maybe_ordered_list_item(
        mut first: char,
        chars: &mut std::str::Chars,
    ) -> Option<(crate::OrderedListNumbering, crate::OrderedListStyle, usize)> {
        fn is_roman_lower_digit(c: char) -> bool {
            matches!(c, 'i' | 'v' | 'x' | 'l' | 'c' | 'd' | 'm')
        }

        fn is_roman_upper_digit(c: char) -> bool {
            matches!(c, 'I' | 'V' | 'X' | 'L' | 'C' | 'D' | 'M')
        }

        let start_paren = first == '(';
        if start_paren {
            first = chars.next().unwrap_or(EOF);
        }

        let numbering = if first.is_ascii_digit() {
            Decimal
        } else if first.is_ascii_lowercase() {
            AlphaLower
        } else if first.is_ascii_uppercase() {
            AlphaUpper
        } else if is_roman_lower_digit(first) {
            RomanLower
        } else if is_roman_upper_digit(first) {
            RomanUpper
        } else {
            return None;
        };

        let chars_num = chars.clone();
        let len_num = 1 + chars_num
            .clone()
            .take_while(|c| match numbering {
                Decimal => c.is_ascii_digit(),
                AlphaLower => c.is_ascii_lowercase(),
                AlphaUpper => c.is_ascii_uppercase(),
                RomanLower => is_roman_lower_digit(*c),
                RomanUpper => is_roman_upper_digit(*c),
            })
            .count();

        let post_num = chars.nth(len_num - 1)?;
        let style = if start_paren {
            if post_num == ')' {
                ParenParen
            } else {
                return None;
            }
        } else if post_num == ')' {
            Paren
        } else if post_num == '.' {
            Period
        } else {
            return None;
        };
        let len_style = usize::from(start_paren) + 1;

        let chars_num = std::iter::once(first).chain(chars_num.take(len_num - 1));
        let numbering = if matches!(numbering, AlphaLower)
            && chars_num.clone().all(is_roman_lower_digit)
        {
            RomanLower
        } else if matches!(numbering, AlphaUpper) && chars_num.clone().all(is_roman_upper_digit) {
            RomanUpper
        } else {
            numbering
        };

        if chars.next().map_or(true, char::is_whitespace) {
            Some((numbering, style, len_num + len_style))
        } else {
            None
        }
    }
}

impl std::fmt::Display for Block {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Block::Atom(a) => std::fmt::Debug::fmt(a, f),
            Block::Leaf(e) => std::fmt::Debug::fmt(e, f),
            Block::Container(c) => std::fmt::Debug::fmt(c, f),
        }
    }
}

impl std::fmt::Display for Atom {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Inline")
    }
}

/// Similar to `std::str::split('\n')` but newline is included and spans are used instead of `str`.
fn lines(src: &str) -> impl Iterator<Item = Span> + '_ {
    let mut chars = src.chars();
    std::iter::from_fn(move || {
        if chars.as_str().is_empty() {
            None
        } else {
            let start = src.len() - chars.as_str().len();
            chars.find(|c| *c == '\n');
            let end = src.len() - chars.as_str().len();
            if start == end {
                None
            } else {
                Some(Span::new(start, end))
            }
        }
    })
}

#[cfg(test)]
mod test {
    use crate::tree::EventKind;
    use crate::tree::EventKind::*;
    use crate::OrderedListNumbering::*;
    use crate::OrderedListStyle::*;

    use super::Atom::*;
    use super::Block;
    use super::Container::*;
    use super::Leaf::*;
    use super::ListType::*;
    use super::Node::*;

    macro_rules! test_parse {
            ($src:expr $(,$($event:expr),* $(,)?)?) => {
                let t = super::TreeParser::new($src).parse();
                let actual = t.map(|ev| (ev.kind, ev.span.of($src))).collect::<Vec<_>>();
                let expected = &[$($($event),*,)?];
                assert_eq!(actual, expected, "\n\n{}\n\n", $src);
            };
        }

    #[test]
    fn parse_para_oneline() {
        test_parse!(
            "para\n",
            (Enter(Leaf(Paragraph)), ""),
            (Inline, "para"),
            (Exit(Leaf(Paragraph)), ""),
        );
    }

    #[test]
    fn parse_para_multiline() {
        test_parse!(
            "para0\npara1\n",
            (Enter(Leaf(Paragraph)), ""),
            (Inline, "para0\n"),
            (Inline, "para1"),
            (Exit(Leaf(Paragraph)), ""),
        );
    }

    #[test]
    fn parse_heading_multi() {
        test_parse!(
            concat!(
                            "# 2\n",
                            "\n",
                            " #   8\n",
                            "  12\n",
                            "15\n", //
                        ),
            (Enter(Leaf(Heading)), "#"),
            (Inline, "2"),
            (Exit(Leaf(Heading)), "#"),
            (Atom(Blankline), "\n"),
            (Enter(Leaf(Heading)), "#"),
            (Inline, "8\n"),
            (Inline, "12\n"),
            (Inline, "15"),
            (Exit(Leaf(Heading)), "#"),
        );
    }

    #[test]
    fn parse_blockquote() {
        test_parse!(
            "> a\n",
            (Enter(Container(Blockquote)), ">"),
            (Enter(Leaf(Paragraph)), ""),
            (Inline, "a"),
            (Exit(Leaf(Paragraph)), ""),
            (Exit(Container(Blockquote)), ">"),
        );
        test_parse!(
            "> a\nb\nc\n",
            (Enter(Container(Blockquote)), ">"),
            (Enter(Leaf(Paragraph)), ""),
            (Inline, "a\n"),
            (Inline, "b\n"),
            (Inline, "c"),
            (Exit(Leaf(Paragraph)), ""),
            (Exit(Container(Blockquote)), ">"),
        );
        test_parse!(
            concat!(
                "> a\n",
                ">\n",
                "> ## hl\n",
                ">\n",
                ">  para\n", //
            ),
            (Enter(Container(Blockquote)), ">"),
            (Enter(Leaf(Paragraph)), ""),
            (Inline, "a"),
            (Exit(Leaf(Paragraph)), ""),
            (Atom(Blankline), "\n"),
            (Enter(Leaf(Heading)), "##"),
            (Inline, "hl"),
            (Exit(Leaf(Heading)), "##"),
            (Atom(Blankline), "\n"),
            (Enter(Leaf(Paragraph)), ""),
            (Inline, "para"),
            (Exit(Leaf(Paragraph)), ""),
            (Exit(Container(Blockquote)), ">"),
        );
    }

    #[test]
    fn parse_blockquote_empty() {
        test_parse!(
            "> \n",
            (Enter(Container(Blockquote)), ">"),
            (EventKind::Atom(Blankline), "\n"),
            (Exit(Container(Blockquote)), ">"),
        );
        test_parse!(
            ">",
            (Enter(Container(Blockquote)), ">"),
            (EventKind::Atom(Blankline), ""),
            (Exit(Container(Blockquote)), ">"),
        );
    }

    #[test]
    fn parse_code_block() {
        test_parse!(
            concat!("```\n", "l0\n"),
            (Enter(Leaf(CodeBlock)), "",),
            (Inline, "l0\n"),
            (Exit(Leaf(CodeBlock)), "",),
        );
        test_parse!(
            concat!(
                "```\n",
                "l0\n",
                "```\n",
                "\n",
                "para\n", //
            ),
            (Enter(Leaf(CodeBlock)), ""),
            (Inline, "l0\n"),
            (Exit(Leaf(CodeBlock)), ""),
            (Atom(Blankline), "\n"),
            (Enter(Leaf(Paragraph)), ""),
            (Inline, "para"),
            (Exit(Leaf(Paragraph)), ""),
        );
        test_parse!(
            concat!(
                "````  lang\n",
                "l0\n",
                "```\n",
                " l1\n",
                "````", //
            ),
            (Enter(Leaf(CodeBlock)), "lang"),
            (Inline, "l0\n"),
            (Inline, "```\n"),
            (Inline, " l1\n"),
            (Exit(Leaf(CodeBlock)), "lang"),
        );
        test_parse!(
            concat!(
                "```\n", //
                "a\n",   //
                "```\n", //
                "```\n", //
                "bbb\n", //
                "```\n", //
            ),
            (Enter(Leaf(CodeBlock)), ""),
            (Inline, "a\n"),
            (Exit(Leaf(CodeBlock)), ""),
            (Enter(Leaf(CodeBlock)), ""),
            (Inline, "bbb\n"),
            (Exit(Leaf(CodeBlock)), ""),
        );
        test_parse!(
            concat!(
                "~~~\n",
                "code\n",
                "  block\n",
                "~~~\n", //
            ),
            (Enter(Leaf(CodeBlock)), ""),
            (Inline, "code\n"),
            (Inline, "  block\n"),
            (Exit(Leaf(CodeBlock)), ""),
        );
    }

    #[test]
    fn parse_link_definition() {
        test_parse!(
            "[tag]: url\n",
            (Enter(Leaf(LinkDefinition)), "tag"),
            (Inline, "url"),
            (Exit(Leaf(LinkDefinition)), "tag"),
        );
    }

    #[test]
    fn parse_footnote() {
        test_parse!(
            "[^tag]: description\n",
            (Enter(Container(Footnote)), "tag"),
            (Enter(Leaf(Paragraph)), ""),
            (Inline, "description"),
            (Exit(Leaf(Paragraph)), ""),
            (Exit(Container(Footnote)), "tag"),
        );
    }

    #[test]
    fn parse_footnote_post() {
        test_parse!(
            concat!(
                "[^a]\n",
                "\n",
                "[^a]: note\n",
                "\n",
                "para\n", //
            ),
            (Enter(Leaf(Paragraph)), ""),
            (Inline, "[^a]"),
            (Exit(Leaf(Paragraph)), ""),
            (Atom(Blankline), "\n"),
            (Enter(Container(Footnote)), "a"),
            (Enter(Leaf(Paragraph)), ""),
            (Inline, "note"),
            (Exit(Leaf(Paragraph)), ""),
            (Atom(Blankline), "\n"),
            (Exit(Container(Footnote)), "a"),
            (Enter(Leaf(Paragraph)), ""),
            (Inline, "para"),
            (Exit(Leaf(Paragraph)), ""),
        );
    }

    #[test]
    fn parse_attr() {
        test_parse!(
            "{.some_class}\npara\n",
            (Atom(Attributes), "{.some_class}\n"),
            (Enter(Leaf(Paragraph)), ""),
            (Inline, "para"),
            (Exit(Leaf(Paragraph)), ""),
        );
    }

    #[test]
    fn parse_list_single_item() {
        test_parse!(
            concat!(
                "- abc\n",
                "\n",
                "\n", //
            ),
            (Enter(Container(List(Unordered(b'-')))), "-"),
            (Enter(Container(ListItem(Unordered(b'-')))), "-"),
            (Enter(Leaf(Paragraph)), ""),
            (Inline, "abc"),
            (Exit(Leaf(Paragraph)), ""),
            (Atom(Blankline), "\n"),
            (Atom(Blankline), "\n"),
            (Exit(Container(ListItem(Unordered(b'-')))), "-"),
            (Exit(Container(List(Unordered(b'-')))), "-"),
        );
    }

    #[test]
    fn parse_list_multi_item() {
        test_parse!(
            "- abc\n\n\n- def\n\n",
            (Enter(Container(List(Unordered(b'-')))), "-"),
            (Enter(Container(ListItem(Unordered(b'-')))), "-"),
            (Enter(Leaf(Paragraph)), ""),
            (Inline, "abc"),
            (Exit(Leaf(Paragraph)), ""),
            (Atom(Blankline), "\n"),
            (Atom(Blankline), "\n"),
            (Exit(Container(ListItem(Unordered(b'-')))), "-"),
            (Enter(Container(ListItem(Unordered(b'-')))), "-"),
            (Enter(Leaf(Paragraph)), ""),
            (Inline, "def"),
            (Exit(Leaf(Paragraph)), ""),
            (Atom(Blankline), "\n"),
            (Exit(Container(ListItem(Unordered(b'-')))), "-"),
            (Exit(Container(List(Unordered(b'-')))), "-"),
        );
    }

    #[test]
    fn parse_list_nest() {
        test_parse!(
            concat!(
                "- a\n",     //
                "\n",        //
                "   - aa\n", //
                "\n",        //
                "\n",        //
                "- b\n",     //
            ),
            (Enter(Container(List(Unordered(b'-')))), "-"),
            (Enter(Container(ListItem(Unordered(b'-')))), "-"),
            (Enter(Leaf(Paragraph)), ""),
            (Inline, "a"),
            (Exit(Leaf(Paragraph)), ""),
            (Atom(Blankline), "\n"),
            (Enter(Container(List(Unordered(b'-')))), "-"),
            (Enter(Container(ListItem(Unordered(b'-')))), "-"),
            (Enter(Leaf(Paragraph)), ""),
            (Inline, "aa"),
            (Exit(Leaf(Paragraph)), ""),
            (Atom(Blankline), "\n"),
            (Atom(Blankline), "\n"),
            (Exit(Container(ListItem(Unordered(b'-')))), "-"),
            (Exit(Container(List(Unordered(b'-')))), "-"),
            (Exit(Container(ListItem(Unordered(b'-')))), "-"),
            (Enter(Container(ListItem(Unordered(b'-')))), "-"),
            (Enter(Leaf(Paragraph)), ""),
            (Inline, "b"),
            (Exit(Leaf(Paragraph)), ""),
            (Exit(Container(ListItem(Unordered(b'-')))), "-"),
            (Exit(Container(List(Unordered(b'-')))), "-"),
        );
    }

    macro_rules! test_block {
        ($src:expr, $kind:expr, $str:expr, $len:expr $(,)?) => {
            let lines = super::lines($src).map(|sp| sp.of($src));
            let (_indent, kind, sp, len) = super::BlockParser::parse(lines).unwrap();
            assert_eq!(
                (kind, sp.of($src), len),
                ($kind, $str, $len),
                "\n\n{}\n\n",
                $src
            );
        };
    }

    #[test]
    fn block_blankline() {
        test_block!("\n", Block::Atom(Blankline), "\n", 1);
        test_block!(" \n", Block::Atom(Blankline), "\n", 1);
    }

    #[test]
    fn block_multiline() {
        test_block!(
            "# heading\n spanning two lines\n",
            Block::Leaf(Heading),
            "#",
            2
        );
    }

    #[test]
    fn block_blockquote() {
        test_block!(
            concat!(
                "> a\n",    //
                ">\n",      //
                "  >  b\n", //
                ">\n",      //
                "> c\n",    //
            ),
            Block::Container(Blockquote),
            ">",
            5,
        );
    }

    #[test]
    fn block_thematic_break() {
        test_block!("---\n", Block::Atom(ThematicBreak), "---", 1);
        test_block!(
            concat!(
                "   -*- -*-\n",
                "\n",
                "para", //
            ),
            Block::Atom(ThematicBreak),
            "-*- -*-",
            1
        );
    }

    #[test]
    fn block_code_block() {
        test_block!(
            concat!(
                "````  lang\n",
                "l0\n",
                "```\n",
                " l1\n",
                "````", //
            ),
            Block::Leaf(CodeBlock),
            "lang",
            5,
        );
        test_block!(
            concat!(
                "```\n", //
                "a\n",   //
                "```\n", //
                "```\n", //
                "bbb\n", //
                "```\n", //
            ),
            Block::Leaf(CodeBlock),
            "",
            3,
        );
        test_block!(
            concat!(
                "``` no space in lang specifier\n",
                "l0\n",
                "```\n", //
            ),
            Block::Leaf(Paragraph),
            "",
            3,
        );
    }

    #[test]
    fn block_link_definition() {
        test_block!("[tag]: url\n", Block::Leaf(LinkDefinition), "tag", 1);
    }

    #[test]
    fn block_link_definition_multiline() {
        test_block!(
            concat!(
                "[tag]: uuu\n",
                " rl\n", //
            ),
            Block::Leaf(LinkDefinition),
            "tag",
            2,
        );
        test_block!(
            concat!(
                "[tag]: url\n",
                "para\n", //
            ),
            Block::Leaf(LinkDefinition),
            "tag",
            1,
        );
    }

    #[test]
    fn block_footnote_empty() {
        test_block!("[^tag]:\n", Block::Container(Footnote), "tag", 1);
    }

    #[test]
    fn block_footnote_single() {
        test_block!("[^tag]: a\n", Block::Container(Footnote), "tag", 1);
    }

    #[test]
    fn block_footnote_multiline() {
        test_block!(
            concat!(
                "[^tag]: a\n",
                " b\n", //
            ),
            Block::Container(Footnote),
            "tag",
            2,
        );
    }

    #[test]
    fn block_footnote_multiline_post() {
        test_block!(
            concat!(
                "[^tag]: a\n",
                " b\n",
                "\n",
                "para\n", //
            ),
            Block::Container(Footnote),
            "tag",
            3,
        );
    }

    #[test]
    fn block_list_bullet() {
        test_block!(
            "- abc\n",
            Block::Container(ListItem(Unordered(b'-'))),
            "-",
            1
        );
        test_block!(
            "+ abc\n",
            Block::Container(ListItem(Unordered(b'+'))),
            "+",
            1
        );
        test_block!(
            "* abc\n",
            Block::Container(ListItem(Unordered(b'*'))),
            "*",
            1
        );
    }

    #[test]
    fn block_list_description() {
        test_block!(": abc\n", Block::Container(ListItem(Description)), ":", 1);
    }

    #[test]
    fn block_list_task() {
        test_block!("- [ ] abc\n", Block::Container(ListItem(Task)), "- [ ]", 1);
        test_block!("+ [x] abc\n", Block::Container(ListItem(Task)), "+ [x]", 1);
        test_block!("* [X] abc\n", Block::Container(ListItem(Task)), "* [X]", 1);
    }

    #[test]
    fn block_list_ordered() {
        test_block!(
            "123. abc\n",
            Block::Container(ListItem(Ordered(Decimal, Period))),
            "123.",
            1
        );
        test_block!(
            "i. abc\n",
            Block::Container(ListItem(Ordered(RomanLower, Period))),
            "i.",
            1
        );
        test_block!(
            "I. abc\n",
            Block::Container(ListItem(Ordered(RomanUpper, Period))),
            "I.",
            1
        );
        test_block!(
            "IJ. abc\n",
            Block::Container(ListItem(Ordered(AlphaUpper, Period))),
            "IJ.",
            1
        );
        test_block!(
            "(a) abc\n",
            Block::Container(ListItem(Ordered(AlphaLower, ParenParen))),
            "(a)",
            1
        );
        test_block!(
            "a) abc\n",
            Block::Container(ListItem(Ordered(AlphaLower, Paren))),
            "a)",
            1
        );
    }
}
