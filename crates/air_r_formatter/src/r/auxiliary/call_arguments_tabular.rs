use crate::prelude::*;
use crate::r::auxiliary::call_arguments::FormatRCallArguments;
use air_r_syntax::{
    AnyRExpression, AnyRValue, RArgument, RArgumentList, RCallArguments, RDoubleValue,
    RIntegerValue, RLanguage, RSyntaxNode, RSyntaxToken, RUnaryExpression,
};

use biome_formatter::{CstFormatContext, FormatElement, RemoveSoftLinesBuffer, format_args, write};
use biome_rowan::{AstSeparatedElement, AstSeparatedList};

const DOT_WIDTH: usize = 1;
const EQUALS_WIDTH: usize = 3; // " = "

#[derive(Debug, Clone)]
struct TableInfo {
    rows: Vec<Vec<ArgData>>,
    cols: Vec<ColumnInfo>,
    remaining: Vec<AstSeparatedElement<RLanguage, RArgument>>,
}

#[derive(Debug, Clone)]
struct ArgData {
    node: RArgument,
    parts: ArgParts,
    separator: Option<RSyntaxToken>,
    is_last_in_list: bool,
}

#[derive(Debug, Clone)]
struct ArgParts {
    name_width: usize,
    kind: ArgKind,
}

#[derive(Debug, Clone)]
enum ArgKind {
    Numeric {
        integer_width: usize,
        fractional_width: Option<usize>,
    },
    Other {
        text: String,
    },
}

#[derive(Debug, Default, Clone)]
struct ColumnInfo {
    // It's possible to have `has_decimal = true` and `max_fractional_part = 0`
    // at the same time, if there is an argument like `1.`
    has_decimal: bool,
    max_width: usize,
    max_name_width: usize,
    max_value_width: usize,
    max_integer_part: usize,
    max_fractional_part: usize,
}

impl FormatRCallArguments {
    pub(crate) fn fmt_tabular(
        &self,
        node: &RCallArguments,
        f: &mut RFormatter,
    ) -> FormatResult<Option<()>> {
        let l_token = node.l_paren_token()?;
        let r_token = node.r_paren_token()?;
        let args = node.items();

        if args.is_empty() {
            return self.fmt_call_like(node, f).map(Some);
        }

        // Get table and alignment info
        let table = match build_table(&args, f)? {
            Some(table) => table,
            None => return Ok(None),
        };
        let rows = table.rows;
        let column_info = table.cols;

        // Format with alignment
        let formatted_table = format_with(|f: &mut RFormatter| {
            for (row_i, row) in rows.iter().enumerate() {
                if row_i > 0 {
                    // Rows are separated with hard line breaks because the
                    // lines of arguments in tabular calls should never be
                    // rearranged by the formatter.
                    write!(f, [hard_line_break()])?;
                }

                for (col_j, arg_data) in row.iter().enumerate() {
                    let name_clause = arg_data.node.name_clause();

                    let (left_pad, right_pad) = if column_info[col_j].max_width > 0 {
                        column_info[col_j].padding(&arg_data.parts)
                    } else {
                        // Empty columns are not padded, so that commas stick to
                        // each other as in regular calls: `list(,,,)`
                        (0, 0)
                    };

                    let max_name_width = column_info[col_j].max_name_width;

                    format_leading_comments(arg_data.node.syntax()).fmt(f)?;
                    disable_skip_comments(arg_data.node.syntax(), f);

                    // Format name and equals sign if present, or add indent for unnamed
                    if let Some(clause) = name_clause {
                        disable_skip_comments(clause.syntax(), f);
                        format_leading_comments(clause.syntax()).fmt(f)?;

                        write!(f, [clause.name()?.format()])?;

                        // Add padding to align equals signs
                        let name_width = arg_data.parts.name_width;
                        let equal_pad = max_name_width.saturating_sub(name_width);
                        write_spaces(equal_pad, f)?;

                        // Format equals sign
                        write!(f, [space(), clause.eq_token()?.format(), space()])?;
                    } else {
                        // Add padding to align unnamed arguments
                        if max_name_width > 0 {
                            let name_pad = max_name_width + EQUALS_WIDTH;
                            write_spaces(name_pad, f)?;
                        }
                    }

                    // Add left padding for right-aligned values
                    write_spaces(left_pad, f)?;

                    // Format the value
                    match &arg_data.parts.kind {
                        ArgKind::Other { text } if !text.is_empty() => {
                            write!(f, [dynamic_text(text, 0.into())])?;
                        }
                        ArgKind::Numeric { .. } => {
                            if let Some(value) = arg_data.node.value() {
                                write!(f, [value.format()])?;
                            }
                        }
                        _ => {} // Empty argument: print nothing
                    }

                    format_trailing_comments(arg_data.node.syntax()).fmt(f)?;

                    // Add right padding (but not for the last item)
                    if !arg_data.is_last_in_list {
                        write_spaces(right_pad, f)?;
                    }

                    // Format separator
                    if let Some(sep) = &arg_data.separator {
                        write!(f, [space(), sep.format()])?;
                    } else if !arg_data.is_last_in_list {
                        // Syntactic invariant: All arguments except the last one have a separator
                        return Err(FormatError::SyntaxError);
                    }

                    // Add space between columns (but only if next column is non-empty)
                    if let Some(next_col) = column_info.get(col_j + 1) {
                        if next_col.max_width > 0 {
                            // This must be a soft space so it can be removed by
                            // the printer if trailing
                            write!(f, [space()])?;
                        }
                    }
                }
            }

            Ok(())
        });

        // Copied from `Format` method for `FormatAllArgsBrokenOut`
        let remaining = format_with(|f| {
            if table.remaining.is_empty() {
                return Ok(());
            }

            // Start on a new line after table
            write!(f, [hard_line_break()])?;

            for entry in table.remaining.iter() {
                let node = entry.node()?;
                let leading_lines = get_lines_before(node.syntax());

                // Respect 1 full empty line between sequential arguments
                // if the user requested it (similar to top level expressions)
                match leading_lines {
                    0 | 1 => write!(f, [soft_line_break_or_space()])?,
                    _ => write!(f, [empty_line()])?,
                }

                write!(f, [node.format(), entry.trailing_separator()?.format()])?;
            }

            Ok(())
        });

        write!(
            f,
            [group(&format_args![
                l_token.format(),
                soft_block_indent(&format_args![&formatted_table, &remaining]),
                r_token.format()
            ])
            .should_expand(true)]
        )?;

        Ok(Some(()))
    }
}

// First pass: Parse arguments into rows and gather column width information for
// alignment. This involves finding the integer and fractional widths of numeric
// arguments, and formatting other arguments in a flat layout to get the width of
// the final printed text.
fn build_table(args: &RArgumentList, f: &mut RFormatter) -> FormatResult<Option<TableInfo>> {
    let mut cols: Vec<ColumnInfo> = Vec::new();
    let mut rows: Vec<Vec<ArgData>> = Vec::new();
    let mut current_row = Vec::new();

    let mut items = args.elements().enumerate();
    let mut remaining = vec![];

    for (i, arg) in &mut items {
        // We've encountered an `off` comment before, keep collecting remaining args
        if remaining.len() > 0 {
            remaining.push(arg);
            continue;
        }

        let arg_node = arg.node()?;
        let arg_separator = arg.trailing_separator()?;

        // If we see an `off` comment, start collecting remaining args
        if crate::comments_directives(arg_node, f).iter().any(|d| {
            matches!(
                d,
                comments::Directive::Format(comments::FormatDirective::Tabular(Some(
                    comments::TabularParam::Off
                )))
            )
        }) {
            remaining.push(arg);
            continue;
        }

        // Detect leading line breaks because they indicate a new row
        let lines_before = if arg_node.value().is_some() {
            get_lines_before(arg_node.syntax())
        } else {
            arg_separator.map_or(0, get_lines_before_token)
        };

        // Push new row if any. Empty lines are not rows. Note empty lines still
        // get formatted as part of trivia in the formatting pass. We just don't
        // want to consider empty lines as rows in our table info.
        if lines_before > 0 && !current_row.is_empty() {
            rows.push(current_row);
            current_row = Vec::new();
        }

        let column_index = current_row.len();
        while cols.len() <= column_index {
            cols.push(ColumnInfo::default());
        }

        let arg_parts = match ArgParts::parse(arg_node, f)? {
            Some(parts) => parts,
            None => return Ok(None),
        };

        let name_width = arg_parts.name_width;
        let value_width = arg_parts.width();

        let total_width = if name_width > 0 {
            name_width + EQUALS_WIDTH + value_width
        } else {
            value_width
        };

        let col = &mut cols[column_index];
        col.max_width = col.max_width.max(total_width);
        col.max_name_width = col.max_name_width.max(name_width);
        col.max_value_width = col.max_value_width.max(value_width);

        if let ArgKind::Numeric {
            integer_width,
            fractional_width,
        } = arg_parts.kind
        {
            col.max_integer_part = col.max_integer_part.max(integer_width);
            if let Some(frac_len) = fractional_width {
                // Mark that this column contains decimals so padding logic can
                // align at the decimal point.
                col.has_decimal = true;
                col.max_fractional_part = col.max_fractional_part.max(frac_len);
            }
        }

        current_row.push(ArgData {
            node: arg_node.clone(),
            parts: arg_parts,
            separator: arg_separator.cloned(),
            is_last_in_list: i == args.len() - 1,
        });
    }

    if !current_row.is_empty() {
        rows.push(current_row);
    }

    Ok(Some(TableInfo {
        rows,
        cols,
        remaining,
    }))
}

impl ColumnInfo {
    fn padding(&self, arg: &ArgParts) -> (usize, usize) {
        if self.has_decimal {
            self.decimal_padding(arg)
        } else {
            self.simple_padding(arg)
        }
    }

    fn simple_padding(&self, arg: &ArgParts) -> (usize, usize) {
        // Non-decimal columns: All arguments are padded to `max_width`
        let padding = self.max_value_width.saturating_sub(arg.width());

        match &arg.kind {
            ArgKind::Numeric { .. } => (padding, 0), // Right-align
            ArgKind::Other { .. } => (0, padding),   // Left-align
        }
    }

    fn decimal_padding(&self, arg: &ArgParts) -> (usize, usize) {
        // In decimal columns, numeric arguments form a nested sub-column
        // where decimal points align. However, `Other` arguments (like
        // `foo()` or `"foo"`) might be wider than this numeric sub-column.
        // To ensure all commas align vertically, we pad all arguments to
        // the width of the widest element, whether that's the numeric
        // sub-column width or a wider `Other` argument.

        // Width of the numeric sub-column (for decimal point alignment)
        let max_decimal_width = self.max_integer_part + DOT_WIDTH + self.max_fractional_part;

        // The width all arguments must reach for commas to align
        let target_width = self.max_value_width.max(max_decimal_width);

        match &arg.kind {
            ArgKind::Numeric {
                integer_width,
                fractional_width,
            } => {
                // Align at decimal point
                let left = self.max_integer_part.saturating_sub(*integer_width);
                let right = match fractional_width {
                    Some(frac) => self.max_fractional_part.saturating_sub(*frac),
                    None => DOT_WIDTH + self.max_fractional_part,
                };

                // Add extra padding if any "Other" argument is wider than
                // decimal alignment
                let extra = target_width.saturating_sub(max_decimal_width);

                (left, right + extra)
            }

            ArgKind::Other { .. } => {
                // Left-align with right padding to reach target width
                let right = target_width.saturating_sub(arg.width());

                (0, right)
            }
        }
    }
}

impl ArgParts {
    fn parse(arg: &RArgument, f: &mut RFormatter) -> FormatResult<Option<Self>> {
        let name_width = match arg.name_clause() {
            Some(clause) => {
                let text = clause.name()?.syntax().text_trimmed();
                usize::from(text.len())
            }
            None => 0,
        };

        let kind = match arg.value() {
            Some(AnyRExpression::AnyRValue(AnyRValue::RIntegerValue(value))) => {
                Some(Self::parse_integer(value)?)
            }
            Some(AnyRExpression::AnyRValue(AnyRValue::RDoubleValue(value))) => {
                Some(Self::parse_decimal(value)?)
            }
            Some(AnyRExpression::RUnaryExpression(value)) => Self::parse_unary(arg, value, f)?,
            Some(AnyRExpression::AnyRValue(AnyRValue::RBogusValue(_))) => {
                return Err(FormatError::SyntaxError);
            }
            _ => Self::parse_other(arg, f)?,
        };

        Ok(kind.map(|kind| ArgParts { name_width, kind }))
    }

    // Delegate to numerical parsing, but add 1 to the integer part for the
    // unary operator. Note that repeated unary operators like `--1` wil fall
    // back to ordinary parsing.
    fn parse_unary(
        arg: &RArgument,
        unary: RUnaryExpression,
        f: &mut RFormatter,
    ) -> FormatResult<Option<ArgKind>> {
        let operator = unary.operator()?;
        let argument = unary.argument()?;

        if !matches!(operator.text_trimmed(), "+" | "-") {
            return Self::parse_other(arg, f);
        }

        let kind = match argument {
            AnyRExpression::AnyRValue(AnyRValue::RIntegerValue(value)) => {
                Self::parse_integer(value)?
            }
            AnyRExpression::AnyRValue(AnyRValue::RDoubleValue(value)) => {
                Self::parse_decimal(value)?
            }
            _ => return Self::parse_other(arg, f),
        };

        let ArgKind::Numeric {
            integer_width,
            fractional_width,
        } = kind
        else {
            unreachable!();
        };

        // `+ 1` to account for unary operator
        return Ok(Some(ArgKind::Numeric {
            integer_width: integer_width + 1,
            fractional_width,
        }));
    }

    fn parse_integer(value: RIntegerValue) -> FormatResult<ArgKind> {
        let token = value.value_token()?;
        let text = token.text_trimmed();

        Ok(ArgKind::Numeric {
            integer_width: usize::from(text.len()),
            fractional_width: None,
        })
    }

    fn parse_decimal(value: RDoubleValue) -> FormatResult<ArgKind> {
        let token = value.value_token()?;
        let text = token.text_trimmed();
        let len = usize::from(text.len());

        let (integer_width, fractional_width) = match text.find('.') {
            Some(pos) => (pos, Some(len - pos - DOT_WIDTH)),
            None => (len, None),
        };

        Ok(ArgKind::Numeric {
            integer_width,
            fractional_width, // Note this might be 0, e.g. `1.`
        })
    }

    fn parse_other(arg: &RArgument, f: &mut RFormatter) -> FormatResult<Option<ArgKind>> {
        disable_skip_comments(arg.syntax(), f);

        let snapshot = f.snapshot();
        let result = (|| {
            // Format with flat layout by disabling soft line breaks
            let mut buffer = RemoveSoftLinesBuffer::new(f);
            let mut recording = buffer.start_recording();

            // Format only the value part (not the name)
            if let Some(value) = arg.value() {
                write!(recording, [value.format()])?;
            }

            let recorded = recording.stop();
            let ir: Vec<FormatElement> = recorded.into_iter().cloned().collect();
            let document = Document::from(ir);

            // Ideally we'd print without cloning the context for every
            // argument. Can we do that? Perhaps with snapshotting?
            let formatted = biome_formatter::Formatted::new(document, f.context().clone());
            let text = formatted.print()?.into_code();

            // `will_break()` should not fail on us since we're formatting with
            // soft breaks diabled, but detecting newlines in the printed output
            // is the most reliable approach. Since we already need the text to
            // compute the argument width, we might as well do that.
            if text.contains('\n') {
                return Ok(None);
            }

            Ok(Some(ArgKind::Other { text }))
        })();

        f.restore_snapshot(snapshot);
        result
    }

    fn width(&self) -> usize {
        match &self.kind {
            ArgKind::Numeric {
                integer_width,
                fractional_width,
            } => match fractional_width {
                Some(frac_len) => *integer_width + DOT_WIDTH + *frac_len,
                None => *integer_width,
            },
            ArgKind::Other { text } => text.len(),
        }
    }
}

/// Apply padding with incompressible whitespace
fn write_spaces(count: usize, f: &mut RFormatter) -> FormatResult<()> {
    if count > 0 {
        write!(f, [dynamic_text(&" ".repeat(count), 0.into())])?;
    }
    Ok(())
}

/// Skip comments nested in the table are invalid, but mark them as checked so the formatter
fn disable_skip_comments(syntax: &RSyntaxNode, f: &RFormatter) {
    f.context().comments().mark_suppression_checked(syntax);
}
