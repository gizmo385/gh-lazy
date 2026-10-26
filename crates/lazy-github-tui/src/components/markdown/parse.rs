//! Source to block tree, over `pulldown-cmark`'s events, reading the document the way markdown-it's `gfm-like` preset
//! and Textual's `Markdown` widget do in Python.

use std::sync::Arc;

use pulldown_cmark::{CodeBlockKind, Event, HeadingLevel, LinkType, OffsetIter, Options, Parser, Tag, TagEnd};

use super::{Block, Inline, InlineStyle, linkify};

/// How deep quotes and lists nest before the rest of a container reads as one paragraph of its text. Layout recurses
/// per level, and no README nests this far.
const MAX_DEPTH: usize = 32;

/// What Textual shows for an image, before its alt text.
const IMAGE_PREFIX: &str = "🖼  ";

pub(super) fn parse(source: &str) -> Vec<Block> {
    // Tables and strikethrough only, as in `gfm-like`: no task lists, footnotes, smart punctuation or heading
    // attributes. Linkify isn't a pulldown-cmark option: `Inlines` splits each text token at its links.
    let options = Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH;
    let mut parser = BlockParser {
        events: Parser::new_ext(source, options).into_offset_iter(),
        source,
        depth: 0,
    };
    parser.blocks(None)
}

struct BlockParser<'a> {
    events: OffsetIter<'a>,
    source: &'a str,
    /// How many quotes and list items enclose the blocks being read.
    depth: usize,
}

impl<'a> BlockParser<'a> {
    /// Reads blocks up to the `end` of their container, or to the end of the input.
    fn blocks(&mut self, end: Option<TagEnd>) -> Vec<Block> {
        let mut blocks = Vec::new();
        // Inline content outside a paragraph: a tight list item's text, which markdown-it puts in a (hidden)
        // paragraph and pulldown-cmark doesn't.
        let mut loose = Inlines::default();
        let flush = |loose: &mut Inlines, blocks: &mut Vec<Block>| {
            if !loose.is_empty() {
                blocks.push(Block::Paragraph(std::mem::take(loose).finish()));
            }
        };
        while let Some((event, range)) = self.events.next() {
            match event {
                Event::End(tag) if Some(tag) == end => break,
                Event::Start(tag) if is_inline(&tag) => loose.start(&tag, &self.source[range]),
                Event::End(tag) if is_inline_end(tag) => loose.end(tag),
                Event::Start(tag) => {
                    flush(&mut loose, &mut blocks);
                    if let Some(block) = self.block(tag) {
                        blocks.push(block);
                    }
                }
                Event::Rule => {
                    flush(&mut loose, &mut blocks);
                    blocks.push(Block::Rule);
                }
                // HTML blocks vanish, as Textual drops markdown-it's `html_block` tokens. Any other stray end can't
                // come from the options enabled.
                Event::Html(_) | Event::End(_) => {}
                event => loose.event(event),
            }
        }
        flush(&mut loose, &mut blocks);
        blocks
    }

    /// The block a non-inline tag starts, read through its end. `None` for what isn't shown.
    fn block(&mut self, tag: Tag<'a>) -> Option<Block> {
        Some(match tag {
            Tag::Paragraph => Block::Paragraph(self.inlines(TagEnd::Paragraph)),
            Tag::Heading { level, .. } => Block::Heading {
                level: heading_level(level),
                text: self.inlines(TagEnd::Heading(level)),
            },
            Tag::BlockQuote(kind) => {
                let end = TagEnd::BlockQuote(kind);
                Block::Quote(self.nested(end))
            }
            Tag::List(start) => self.list(start),
            Tag::CodeBlock(kind) => self.code(kind),
            Tag::Table(_) => self.table(),
            other => {
                // An HTML block, or an item outside a list, which pulldown-cmark doesn't emit: skip it whole.
                self.skip(other.to_end());
                return None;
            }
        })
    }

    /// A container's blocks. Past [`MAX_DEPTH`], its text as one paragraph.
    fn nested(&mut self, end: TagEnd) -> Vec<Block> {
        if self.depth >= MAX_DEPTH {
            return vec![Block::Paragraph(self.flatten(end))];
        }
        self.depth += 1;
        let blocks = self.blocks(Some(end));
        self.depth -= 1;
        blocks
    }

    fn list(&mut self, start: Option<u64>) -> Block {
        let end = TagEnd::List(start.is_some());
        let mut items = Vec::new();
        while let Some((event, _)) = self.events.next() {
            match event {
                Event::Start(Tag::Item) => items.push(self.nested(TagEnd::Item)),
                Event::End(tag) if tag == end => break,
                // A list holds only items.
                _ => {}
            }
        }
        Block::List { start, items }
    }

    fn code(&mut self, kind: CodeBlockKind<'a>) -> Block {
        let language = match kind {
            CodeBlockKind::Fenced(info) if !info.trim().is_empty() => Some(info.trim().to_owned()),
            _ => None,
        };
        let mut text = String::new();
        for (event, _) in self.events.by_ref() {
            match event {
                Event::Text(chunk) => text.push_str(&chunk),
                Event::End(TagEnd::CodeBlock) => break,
                _ => {}
            }
        }
        Block::Code { language, text }
    }

    fn table(&mut self) -> Block {
        let mut head = Vec::new();
        let mut rows = Vec::new();
        let mut row = Vec::new();
        while let Some((event, _)) = self.events.next() {
            match event {
                Event::Start(Tag::TableCell) => row.push(self.inlines(TagEnd::TableCell)),
                Event::End(TagEnd::TableHead) => head = std::mem::take(&mut row),
                Event::End(TagEnd::TableRow) => rows.push(std::mem::take(&mut row)),
                Event::End(TagEnd::Table) => break,
                _ => {}
            }
        }
        // As GFM: every row has the header's cells, short rows padded with empty ones and extra cells dropped.
        for row in &mut rows {
            row.resize_with(head.len(), Vec::new);
        }
        Block::Table { head, rows }
    }

    /// Inline content up to `end`.
    fn inlines(&mut self, end: TagEnd) -> Vec<Inline> {
        let mut inlines = Inlines::default();
        for (event, range) in self.events.by_ref() {
            match event {
                Event::End(tag) if tag == end => break,
                Event::Start(tag) => inlines.start(&tag, &self.source[range]),
                Event::End(tag) => inlines.end(tag),
                event => inlines.event(event),
            }
        }
        inlines.finish()
    }

    /// The plain text of everything up to `end`, as one run.
    fn flatten(&mut self, end: TagEnd) -> Vec<Inline> {
        let mut inlines = Inlines::default();
        let mut open = 0_usize;
        for (event, _) in self.events.by_ref() {
            match event {
                Event::End(tag) if open == 0 && tag == end => break,
                Event::Start(_) => open += 1,
                Event::End(_) => open = open.saturating_sub(1),
                Event::Text(text) | Event::Code(text) => inlines.text(&text, true),
                Event::SoftBreak | Event::HardBreak => inlines.text(" ", true),
                _ => {}
            }
        }
        inlines.finish()
    }

    /// Skips everything up to `end`.
    fn skip(&mut self, end: TagEnd) {
        let mut open = 0_usize;
        for (event, _) in self.events.by_ref() {
            match event {
                Event::End(tag) if open == 0 && tag == end => break,
                Event::Start(_) => open += 1,
                Event::End(_) => open = open.saturating_sub(1),
                _ => {}
            }
        }
    }
}

fn is_inline(tag: &Tag<'_>) -> bool {
    matches!(
        tag,
        Tag::Emphasis | Tag::Strong | Tag::Strikethrough | Tag::Link { .. } | Tag::Image { .. }
    )
}

fn is_inline_end(tag: TagEnd) -> bool {
    matches!(
        tag,
        TagEnd::Emphasis | TagEnd::Strong | TagEnd::Strikethrough | TagEnd::Link | TagEnd::Image
    )
}

fn heading_level(level: HeadingLevel) -> u8 {
    match level {
        HeadingLevel::H1 => 1,
        HeadingLevel::H2 => 2,
        HeadingLevel::H3 => 3,
        HeadingLevel::H4 => 4,
        HeadingLevel::H5 => 5,
        HeadingLevel::H6 => 6,
    }
}

/// Builds the inline runs of one block, as Textual's `_token_to_content` turns markdown-it's inline tokens into
/// styled text.
#[derive(Debug, Default)]
struct Inlines {
    runs: Vec<Inline>,
    strong: usize,
    emphasis: usize,
    /// The open strikethroughs. pulldown-cmark also strikes `~text~`, which markdown-it leaves as text, so each
    /// records whether it's a real `~~` one, and whether it opened outside an image and so styles its text.
    strikes: Vec<Strike>,
    links: Vec<Arc<str>>,
    /// How many images enclose the current text. An image's alt text is plain: Textual adds its tokens' text without
    /// their styles.
    images: usize,
    /// Whether the next text event continues the last one, as one markdown-it text token. Anything in between, even
    /// dropped inline HTML, makes it a token of its own, whose whitespace collapses separately.
    joins: bool,
    /// The current text token's text, outside links and images, held until the token ends so linkify can look for
    /// bare URLs in all of it, as markdown-it's linkify does per text token.
    pending: String,
}

#[derive(Debug, Clone, Copy)]
struct Strike {
    real: bool,
    styled: bool,
}

impl Inlines {
    fn is_empty(&self) -> bool {
        self.runs.is_empty() && self.pending.is_empty()
    }

    fn finish(mut self) -> Vec<Inline> {
        self.flush();
        self.runs
    }

    fn style(&self) -> InlineStyle {
        InlineStyle {
            strong: self.strong > 0,
            emphasis: self.emphasis > 0,
            strike: self.strikes.iter().any(|s| s.real && s.styled),
            code: false,
        }
    }

    fn start(&mut self, tag: &Tag<'_>, source: &str) {
        self.flush();
        self.joins = false;
        match tag {
            Tag::Strikethrough => {
                let strike = Strike {
                    real: source.starts_with("~~"),
                    styled: self.images == 0,
                };
                if !strike.real {
                    self.text("~", false);
                }
                self.strikes.push(strike);
            }
            Tag::Image { dest_url, .. } => {
                if self.images == 0 {
                    self.links.push(Arc::from(dest_url.as_ref()));
                    self.text(IMAGE_PREFIX, false);
                }
                self.images += 1;
            }
            // Inside an image, only the text counts.
            _ if self.images > 0 => {}
            Tag::Strong => self.strong += 1,
            Tag::Emphasis => self.emphasis += 1,
            // markdown-it's email autolinks (`<a@b.co>`) go to `mailto:`; pulldown-cmark gives the bare address.
            Tag::Link {
                link_type: LinkType::Email,
                dest_url,
                ..
            } if !dest_url.starts_with("mailto:") => self.links.push(Arc::from(format!("mailto:{dest_url}"))),
            Tag::Link { dest_url, .. } => self.links.push(Arc::from(dest_url.as_ref())),
            _ => {}
        }
    }

    fn end(&mut self, tag: TagEnd) {
        self.flush();
        self.joins = false;
        match tag {
            TagEnd::Strikethrough => {
                if let Some(strike) = self.strikes.pop()
                    && !strike.real
                {
                    self.text("~", false);
                }
            }
            TagEnd::Image => {
                self.images = self.images.saturating_sub(1);
                if self.images == 0 {
                    self.links.pop();
                }
            }
            _ if self.images > 0 => {}
            TagEnd::Strong => self.strong = self.strong.saturating_sub(1),
            TagEnd::Emphasis => self.emphasis = self.emphasis.saturating_sub(1),
            TagEnd::Link => {
                self.links.pop();
            }
            _ => {}
        }
    }

    fn event(&mut self, event: Event<'_>) {
        let in_image = self.images > 0;
        if !matches!(event, Event::Text(_)) {
            self.flush();
            self.joins = false;
        }
        match event {
            Event::Text(text) => self.text(&text, !in_image),
            Event::Code(code) if in_image => self.text(&code, false),
            Event::Code(code) => self.push(
                code.as_ref(),
                InlineStyle {
                    code: true,
                    ..self.style()
                },
            ),
            // Inside an image Textual adds each token's `content`, which is empty for a break and the markup for
            // inline HTML.
            Event::InlineHtml(html) if in_image => self.text(&html, false),
            Event::SoftBreak if !in_image => self.text(" ", false),
            Event::HardBreak if !in_image => self.text("\n", false),
            // Inline HTML loses its tags; the text between them comes as text events.
            _ => {}
        }
    }

    /// Adds text in the current style. `collapse` turns each run of whitespace into one space, as Textual does with
    /// a text token (markdown-it joins adjacent text, so a run that continues the previous one collapses with it).
    /// Collapsed text outside links and images waits in [`Inlines::pending`] for linkify.
    fn text(&mut self, text: &str, collapse: bool) {
        if !collapse || !self.joins {
            self.flush();
        }
        if !collapse {
            self.push(text, self.style());
            self.joins = false;
            return;
        }
        let after_space = self.joins
            && if self.pending.is_empty() {
                self.runs.last().is_some_and(|run| run.text.ends_with(is_space))
            } else {
                self.pending.ends_with(is_space)
            };
        let mut collapsed = String::with_capacity(text.len());
        let mut space = after_space;
        for c in text.chars() {
            if is_space(c) {
                if !space {
                    collapsed.push(' ');
                }
                space = true;
            } else {
                collapsed.push(c);
                space = false;
            }
        }
        if self.images == 0 && self.links.is_empty() {
            self.pending.push_str(&collapsed);
        } else {
            self.push(&collapsed, self.style());
        }
        self.joins = true;
    }

    /// Adds the pending text token, its bare URLs, `www.` hosts and emails as links (#218).
    fn flush(&mut self) {
        if self.pending.is_empty() {
            return;
        }
        let text = std::mem::take(&mut self.pending);
        let style = self.style();
        let mut at = 0;
        for found in linkify::links(&text) {
            self.push_run(&text[at..found.range.start], style, None);
            self.push_run(&text[found.range.clone()], style, Some(Arc::from(found.url)));
            at = found.range.end;
        }
        self.push_run(&text[at..], style, None);
    }

    /// Appends `text` as a run of the link being read, if any.
    fn push(&mut self, text: &str, style: InlineStyle) {
        let link = self.links.last().cloned();
        self.push_run(text, style, link);
    }

    /// Appends `text` as a run, joining it to the last run when they look the same.
    fn push_run(&mut self, text: &str, style: InlineStyle, link: Option<Arc<str>>) {
        if text.is_empty() {
            return;
        }
        if let Some(last) = self.runs.last_mut()
            && !style.code
            && last.style == style
            && same_link(last.link.as_ref(), link.as_ref())
        {
            last.text.push_str(text);
            return;
        }
        self.runs.push(Inline {
            text: text.to_owned(),
            style,
            link,
        });
    }
}

/// Whether two runs belong to the same link (or both to none). Two links to the same place are still two links.
fn same_link(a: Option<&Arc<str>>, b: Option<&Arc<str>>) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(a), Some(b)) => Arc::ptr_eq(a, b),
        _ => false,
    }
}

/// Python's `\s` on `str`: Unicode whitespace, which also counts the four information separators.
pub(super) fn is_space(c: char) -> bool {
    c.is_whitespace() || ('\u{1c}'..='\u{1f}').contains(&c)
}
