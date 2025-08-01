use crate::{
    bibs::BibData,
    buffer::{BufTy, GlobalBuffer},
    char_info::{LexClass, CHAR_WIDTH},
    cite::CiteInfo,
    entries::{EntryData, ENT_STR_SIZE},
    global::{GlobalData, GLOB_STR_SIZE},
    hash::{BstBuiltin, BstFn, HashData, HashExtra},
    log::{
        brace_lvl_one_letters_complaint, braces_unbalanced_complaint,
        bst_1print_string_size_exceeded, bst_2print_string_size_exceeded,
        bst_cant_mess_with_entries_print, output_bbl_line, print_a_pool_str, print_confusion,
        print_fn_class,
    },
    pool::{Checkpoint, StrNumber, StringPool, MAX_PRINT_LINE, MIN_PRINT_LINE},
    scan::{
        check_brace_level, decr_brace_level, enough_text_chars, name_scan_for_and,
        von_name_ends_and_last_name_starts_stuff, von_token_found, QUOTE_NEXT_FN,
    },
    ASCIICode, Bibtex, BibtexError, BufPointer, GlobalItems, HashPointer, StrIlk,
};
use std::ops::{Deref, DerefMut, Index};

#[derive(Copy, Clone, Debug, PartialEq)]
pub(crate) enum ControlSeq {
    LowerI,
    LowerJ,
    LowerAA,
    UpperAA,
    LowerAE,
    UpperAE,
    LowerOE,
    UpperOE,
    LowerO,
    UpperO,
    LowerL,
    UpperL,
    LowerSS,
}

#[derive(Copy, Clone, PartialEq, Eq)]
pub(crate) enum StkType {
    Integer,
    String,
    Function,
    Missing,
    Illegal,
}

#[derive(Copy, Clone, Debug, PartialEq)]
pub(crate) enum ExecVal {
    Integer(i64),
    String(StrNumber),
    Function(HashPointer),
    Missing(StrNumber),
    Illegal,
}

impl ExecVal {
    pub fn ty(&self) -> StkType {
        match self {
            ExecVal::Integer(_) => StkType::Integer,
            ExecVal::String(_) => StkType::String,
            ExecVal::Function(_) => StkType::Function,
            ExecVal::Missing(_) => StkType::Missing,
            ExecVal::Illegal => StkType::Illegal,
        }
    }
}

pub(crate) struct ExecCtx<'a, 'bib, 'cbs> {
    glbl_ctx: &'a mut Bibtex<'bib, 'cbs>,
    pub default: HashPointer,
    pub(crate) lit_stack: Vec<ExecVal>,
    pub mess_with_entries: bool,
    /// Pointer to the current top of the string pool, used to optimize certain string operations
    pub checkpoint: Checkpoint,
}

impl<'a, 'bib, 'cbs> ExecCtx<'a, 'bib, 'cbs> {
    pub(crate) fn new(glbl_ctx: &'a mut Bibtex<'bib, 'cbs>) -> ExecCtx<'a, 'bib, 'cbs> {
        ExecCtx {
            glbl_ctx,
            default: 0,
            lit_stack: Vec::new(),
            mess_with_entries: false,
            checkpoint: Checkpoint::default(),
        }
    }

    pub(crate) fn push_stack(&mut self, val: ExecVal) {
        self.lit_stack.push(val);
    }

    pub(crate) fn pop_stack(
        &mut self,
        pool: &mut StringPool,
        cites: &CiteInfo,
    ) -> Result<ExecVal, BibtexError> {
        if let Some(pop) = self.lit_stack.pop() {
            if let ExecVal::String(str) = pop {
                if self.checkpoint.is_before(str) && !pool.remove_last_str(str) {
                    self.write_logs("Nontop top of string stack");
                    print_confusion(self);
                    return Err(BibtexError::Fatal);
                }
            }
            Ok(pop)
        } else {
            self.write_logs("You can't pop an empty literal stack");
            bst_ex_warn_print(self, pool, cites)?;
            Ok(ExecVal::Illegal)
        }
    }
}

impl<'bib, 'cbs> Deref for ExecCtx<'_, 'bib, 'cbs> {
    type Target = Bibtex<'bib, 'cbs>;

    fn deref(&self) -> &Self::Target {
        self.glbl_ctx
    }
}

impl DerefMut for ExecCtx<'_, '_, '_> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.glbl_ctx
    }
}

pub(crate) fn print_lit(
    ctx: &mut Bibtex<'_, '_>,
    pool: &StringPool,
    hash: &HashData,
    val: ExecVal,
) -> Result<(), BibtexError> {
    match val {
        ExecVal::Integer(val) => {
            ctx.write_logs(&format!("{val}\n"));
        }
        ExecVal::String(str) => {
            print_a_pool_str(ctx, str, pool)?;
            ctx.write_logs("\n");
        }
        ExecVal::Function(f) => {
            print_a_pool_str(ctx, hash.text(f), pool)?;
            ctx.write_logs("\n");
        }
        ExecVal::Missing(s) => {
            print_a_pool_str(ctx, s, pool)?;
            ctx.write_logs("\n");
        }
        ExecVal::Illegal => {
            illegal_literal_confusion(ctx);
            return Err(BibtexError::Fatal);
        }
    }
    Ok(())
}

pub(crate) fn print_stk_lit(
    ctx: &mut Bibtex<'_, '_>,
    pool: &StringPool,
    hash: &HashData,
    val: ExecVal,
) -> Result<(), BibtexError> {
    match val {
        ExecVal::Integer(val) => ctx.write_logs(&format!("{val} is an integer literal")),
        ExecVal::String(str) => {
            ctx.write_logs("\"");
            print_a_pool_str(ctx, str, pool)?;
            ctx.write_logs("\" is a string literal");
        }
        ExecVal::Function(f) => {
            ctx.write_logs("`");
            print_a_pool_str(ctx, hash.text(f), pool)?;
            ctx.write_logs("` is a function literal");
        }
        ExecVal::Missing(s) => {
            ctx.write_logs("`");
            print_a_pool_str(ctx, s, pool)?;
            ctx.write_logs("` is a missing field");
        }
        ExecVal::Illegal => {
            illegal_literal_confusion(ctx);
            return Err(BibtexError::Fatal);
        }
    }
    Ok(())
}

pub(crate) fn print_wrong_stk_lit(
    ctx: &mut ExecCtx<'_, '_, '_>,
    pool: &StringPool,
    hash: &HashData,
    cites: &CiteInfo,
    val: ExecVal,
    typ2: StkType,
) -> Result<(), BibtexError> {
    match val {
        ExecVal::Illegal => Ok(()),
        _ => {
            print_stk_lit(ctx, pool, hash, val)?;

            match typ2 {
                StkType::Integer => ctx.write_logs(", not an integer,"),
                StkType::String => ctx.write_logs(", not a string,"),
                StkType::Function => ctx.write_logs(", not a function,"),
                StkType::Missing | StkType::Illegal => {
                    illegal_literal_confusion(ctx);
                    return Err(BibtexError::Fatal);
                }
            };

            bst_ex_warn_print(ctx, pool, cites)
        }
    }
}

pub(crate) fn bst_ex_warn_print(
    ctx: &mut ExecCtx<'_, '_, '_>,
    pool: &StringPool,
    cites: &CiteInfo,
) -> Result<(), BibtexError> {
    if ctx.mess_with_entries {
        ctx.write_logs(" for entry ");
        print_a_pool_str(ctx, cites.get_cite(cites.ptr()), pool)?;
    }

    ctx.write_logs("\nwhile executing-");
    bst_ln_num_print(ctx, pool)?;
    ctx.mark_error();
    Ok(())
}

pub(crate) fn bst_ln_num_print(
    ctx: &mut Bibtex<'_, '_>,
    pool: &StringPool,
) -> Result<(), BibtexError> {
    ctx.write_logs(&format!(
        "--line {} of file ",
        ctx.bst.as_ref().unwrap().line
    ));
    print_bst_name(ctx, pool, ctx.bst.as_ref().unwrap().name)
}

pub(crate) fn print_bst_name(
    ctx: &mut Bibtex<'_, '_>,
    pool: &StringPool,
    name: StrNumber,
) -> Result<(), BibtexError> {
    print_a_pool_str(ctx, name, pool)?;
    ctx.write_logs(".bst\n");
    Ok(())
}

pub fn illegal_literal_confusion(ctx: &mut Bibtex<'_, '_>) {
    ctx.write_logs("Illegal literal type");
    print_confusion(ctx);
}

fn pop_top_and_print(
    ctx: &mut ExecCtx<'_, '_, '_>,
    pool: &mut StringPool,
    hash: &HashData,
    cites: &CiteInfo,
) -> Result<(), BibtexError> {
    ctx.pop_stack(pool, cites).and_then(|val| {
        if let ExecVal::Illegal = val {
            ctx.write_logs("Empty literal\n");
            Ok(())
        } else {
            print_lit(ctx, pool, hash, val)
        }
    })
}

fn pop_whole_stack(
    ctx: &mut ExecCtx<'_, '_, '_>,
    pool: &mut StringPool,
    hash: &HashData,
    cites: &CiteInfo,
) -> Result<(), BibtexError> {
    while !ctx.lit_stack.is_empty() {
        pop_top_and_print(ctx, pool, hash, cites)?;
    }
    Ok(())
}

pub fn skip_brace_level_greater_than_one(str: &[ASCIICode], brace_level: &mut i32) -> usize {
    let mut pos = 0;
    while *brace_level > 1 && pos < str.len() {
        if str[pos] == b'}' {
            *brace_level -= 1;
        } else if str[pos] == b'{' {
            *brace_level += 1;
        }
        pos += 1;
    }
    pos
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn figure_out_the_formatted_name(
    ctx: &mut ExecCtx<'_, '_, '_>,
    buffers: &mut GlobalBuffer,
    pool: &StringPool,
    cites: &CiteInfo,
    s1: StrNumber,
    first_start: BufPointer,
    first_end: BufPointer,
    last_end: BufPointer,
    von_start: BufPointer,
    von_end: BufPointer,
    name_bf_ptr: &mut BufPointer,
    name_bf_xptr: &mut BufPointer,
    jr_end: BufPointer,
    brace_level: &mut i32,
) -> Result<(), BibtexError> {
    let mut old_idx;
    let mut inner_brace_level = 0;
    let str = pool.get_str(s1);
    let mut idx = 0;

    buffers.set_offset(BufTy::Ex, 1, 0);

    while idx < str.len() {
        if str[idx] == b'{' {
            inner_brace_level += 1;
            idx += 1;
            old_idx = idx;

            let mut alpha_found = false;
            let mut double_letter = false;
            let mut end_of_group = false;
            let mut to_be_written = true;
            let mut cur_token = 0;
            let mut last_token = 0;

            while !end_of_group && idx < str.len() {
                if LexClass::of(str[idx]) == LexClass::Alpha {
                    idx += 1;
                    if alpha_found {
                        brace_lvl_one_letters_complaint(ctx, pool, cites, s1)?;
                        to_be_written = false;
                    } else {
                        match str[idx - 1] {
                            b'f' | b'F' => {
                                cur_token = first_start;
                                last_token = first_end;
                                if cur_token == last_token {
                                    to_be_written = false;
                                }
                                if str[idx] == b'f' || str[idx] == b'F' {
                                    double_letter = true;
                                }
                            }
                            b'v' | b'V' => {
                                cur_token = von_start;
                                last_token = von_end;
                                if cur_token == last_token {
                                    to_be_written = false;
                                }
                                if str[idx] == b'v' || str[idx] == b'V' {
                                    double_letter = true;
                                }
                            }
                            b'l' | b'L' => {
                                cur_token = von_end;
                                last_token = last_end;
                                if cur_token == last_token {
                                    to_be_written = false;
                                }
                                if str[idx] == b'l' || str[idx] == b'L' {
                                    double_letter = true;
                                }
                            }
                            b'j' | b'J' => {
                                cur_token = last_end;
                                last_token = jr_end;
                                if cur_token == last_token {
                                    to_be_written = false;
                                }
                                if str[idx] == b'j' || str[idx] == b'J' {
                                    double_letter = true;
                                }
                            }
                            _ => {
                                brace_lvl_one_letters_complaint(ctx, pool, cites, s1)?;
                                to_be_written = false;
                                break;
                            }
                        }
                        if double_letter {
                            idx += 1;
                        }
                    }
                    alpha_found = true;
                } else if str[idx] == b'}' {
                    inner_brace_level -= 1;
                    idx += 1;
                    end_of_group = true;
                } else if str[idx] == b'{' {
                    inner_brace_level += 1;
                    idx =
                        skip_brace_level_greater_than_one(&str[idx + 1..], &mut inner_brace_level)
                            + idx;
                    idx += 1;
                } else {
                    idx += 1;
                }
            }

            if end_of_group && to_be_written {
                let buf_ptr = buffers.offset(BufTy::Ex, 1);
                idx = old_idx;
                inner_brace_level = 1;
                while inner_brace_level > 0 {
                    if LexClass::of(str[idx]) == LexClass::Alpha && inner_brace_level == 1 {
                        idx += 1;
                        if double_letter {
                            idx += 1;
                        }
                        let mut use_default = true;
                        let mut sp_xptr2 = idx;
                        if str[idx] == b'{' {
                            use_default = false;
                            inner_brace_level += 1;
                            idx += 1;
                            old_idx = idx;
                            idx = skip_brace_level_greater_than_one(
                                &str[idx..],
                                &mut inner_brace_level,
                            ) + idx;
                            sp_xptr2 = idx - 1;
                        }
                        while cur_token < last_token {
                            *name_bf_ptr = buffers.name_tok(cur_token);
                            *name_bf_xptr = buffers.name_tok(cur_token + 1);
                            if double_letter {
                                if buffers.init(BufTy::Ex) + (*name_bf_xptr - *name_bf_ptr)
                                    > buffers.len()
                                {
                                    buffers.grow_all();
                                }
                                let ptr = buffers.offset(BufTy::Ex, 1);
                                let len = *name_bf_xptr - *name_bf_ptr;
                                buffers.copy_within(BufTy::Sv, BufTy::Ex, *name_bf_ptr, ptr, len);
                                buffers.set_offset(BufTy::Ex, 1, ptr + len);
                                *name_bf_ptr += len;
                            } else {
                                while *name_bf_ptr < *name_bf_xptr {
                                    if LexClass::of(buffers.at(BufTy::Sv, *name_bf_ptr))
                                        == LexClass::Alpha
                                    {
                                        if buffers.offset(BufTy::Ex, 1) == buffers.len() {
                                            buffers.grow_all();
                                        }
                                        buffers.set_at(
                                            BufTy::Ex,
                                            buffers.offset(BufTy::Ex, 1),
                                            buffers.at(BufTy::Sv, *name_bf_ptr),
                                        );
                                        buffers.set_offset(
                                            BufTy::Ex,
                                            1,
                                            buffers.offset(BufTy::Ex, 1) + 1,
                                        );
                                        break;
                                    } else if *name_bf_ptr + 1 < *name_bf_xptr
                                        && buffers.at(BufTy::Sv, *name_bf_ptr) == b'{'
                                        && buffers.at(BufTy::Sv, *name_bf_ptr + 1) == b'\\'
                                    {
                                        if buffers.offset(BufTy::Ex, 1) + 2 > buffers.len() {
                                            buffers.grow_all();
                                        }
                                        let offset = buffers.offset(BufTy::Ex, 1);
                                        buffers.set_at(BufTy::Ex, offset, b'{');
                                        buffers.set_at(BufTy::Ex, offset + 1, b'\\');
                                        buffers.set_offset(BufTy::Ex, 1, offset + 2);
                                        *name_bf_ptr += 2;
                                        let mut nm_brace_level = 1;
                                        while *name_bf_ptr < *name_bf_xptr && nm_brace_level > 0 {
                                            if buffers.at(BufTy::Sv, *name_bf_ptr) == b'}' {
                                                nm_brace_level -= 1;
                                            } else if buffers.at(BufTy::Sv, *name_bf_ptr) == b'{' {
                                                nm_brace_level += 1;
                                            }

                                            if buffers.offset(BufTy::Ex, 1) == buffers.len() {
                                                buffers.grow_all();
                                            }

                                            buffers.set_at(
                                                BufTy::Ex,
                                                buffers.offset(BufTy::Ex, 1),
                                                buffers.at(BufTy::Sv, *name_bf_ptr),
                                            );
                                            buffers.set_offset(
                                                BufTy::Ex,
                                                1,
                                                buffers.offset(BufTy::Ex, 1) + 1,
                                            );
                                            *name_bf_ptr += 1;
                                        }
                                        break;
                                    }
                                    *name_bf_ptr += 1;
                                }
                            }

                            cur_token += 1;
                            if cur_token < last_token {
                                if use_default {
                                    if !double_letter {
                                        if buffers.offset(BufTy::Ex, 1) == buffers.len() {
                                            buffers.grow_all();
                                        }
                                        buffers.set_at(
                                            BufTy::Ex,
                                            buffers.offset(BufTy::Ex, 1),
                                            b'.',
                                        );
                                        buffers.set_offset(
                                            BufTy::Ex,
                                            1,
                                            buffers.offset(BufTy::Ex, 1) + 1,
                                        );
                                    }

                                    if buffers.offset(BufTy::Ex, 1) == buffers.len() {
                                        buffers.grow_all();
                                    }

                                    let c = if LexClass::of(buffers.at(BufTy::NameSep, cur_token))
                                        == LexClass::Sep
                                    {
                                        buffers.at(BufTy::NameSep, cur_token)
                                    } else if cur_token == last_token - 1
                                        || (!enough_text_chars(buffers, 3, buf_ptr, brace_level))
                                    {
                                        b'~'
                                    } else {
                                        b' '
                                    };
                                    buffers.set_at(BufTy::Ex, buffers.offset(BufTy::Ex, 1), c);
                                    buffers.set_offset(
                                        BufTy::Ex,
                                        1,
                                        buffers.offset(BufTy::Ex, 1) + 1,
                                    );
                                } else {
                                    if buffers.offset(BufTy::Ex, 1) + (sp_xptr2 - old_idx)
                                        > buffers.len()
                                    {
                                        buffers.grow_all();
                                    }

                                    let ptr = buffers.offset(BufTy::Ex, 1);
                                    let tmp_str = &str[old_idx..sp_xptr2];
                                    buffers.copy_from(BufTy::Ex, ptr, tmp_str);
                                    buffers.set_offset(BufTy::Ex, 1, ptr + tmp_str.len());
                                    idx = sp_xptr2;
                                }
                            }
                        }
                        if !use_default {
                            idx = sp_xptr2 + 1;
                        }
                    } else if str[idx] == b'}' {
                        inner_brace_level -= 1;
                        idx += 1;
                        if inner_brace_level > 0 {
                            if buffers.offset(BufTy::Ex, 1) == buffers.len() {
                                buffers.grow_all();
                            }
                            buffers.set_at(BufTy::Ex, buffers.offset(BufTy::Ex, 1), b'}');
                            buffers.set_offset(BufTy::Ex, 1, buffers.offset(BufTy::Ex, 1) + 1);
                        }
                    } else if str[idx] == b'{' {
                        inner_brace_level += 1;
                        idx += 1;
                        if buffers.offset(BufTy::Ex, 1) == buffers.len() {
                            buffers.grow_all();
                        }
                        buffers.set_at(BufTy::Ex, buffers.offset(BufTy::Ex, 1), b'{');
                        buffers.set_offset(BufTy::Ex, 1, buffers.offset(BufTy::Ex, 1) + 1);
                    } else {
                        if buffers.offset(BufTy::Ex, 1) == buffers.len() {
                            buffers.grow_all();
                        }
                        buffers.set_at(BufTy::Ex, buffers.offset(BufTy::Ex, 1), str[idx]);
                        buffers.set_offset(BufTy::Ex, 1, buffers.offset(BufTy::Ex, 1) + 1);
                        idx += 1;
                    }
                }
                if buffers.offset(BufTy::Ex, 1) > 0
                    && buffers.at(BufTy::Ex, buffers.offset(BufTy::Ex, 1) - 1) == b'~'
                {
                    buffers.set_offset(BufTy::Ex, 1, buffers.offset(BufTy::Ex, 1) - 1);
                    if buffers.at(BufTy::Ex, buffers.offset(BufTy::Ex, 1) - 1) == b'~' {
                    } else if !enough_text_chars(buffers, 3, buf_ptr, brace_level) {
                        buffers.set_offset(BufTy::Ex, 1, buffers.offset(BufTy::Ex, 1) + 1);
                    } else {
                        buffers.set_at(BufTy::Ex, buffers.offset(BufTy::Ex, 1), b' ');
                        buffers.set_offset(BufTy::Ex, 1, buffers.offset(BufTy::Ex, 1) + 1);
                    }
                }
            }
        } else if str[idx] == b'}' {
            braces_unbalanced_complaint(ctx, pool, cites, s1)?;
            idx += 1;
        } else {
            if buffers.offset(BufTy::Ex, 1) == buffers.len() {
                buffers.grow_all();
            }
            buffers.set_at(BufTy::Ex, buffers.offset(BufTy::Ex, 1), str[idx]);
            buffers.set_offset(BufTy::Ex, 1, buffers.offset(BufTy::Ex, 1) + 1);
            idx += 1;
        }
    }

    if inner_brace_level > 0 {
        braces_unbalanced_complaint(ctx, pool, cites, s1)?;
    }

    buffers.set_init(BufTy::Ex, buffers.offset(BufTy::Ex, 1));

    Ok(())
}

pub(crate) fn add_buf_pool(pool: &StringPool, buffers: &mut GlobalBuffer, str: StrNumber) {
    let str = pool.get_str(str);

    if buffers.init(BufTy::Ex) + str.len() > buffers.len() {
        buffers.grow_all();
    }

    let start = buffers.init(BufTy::Ex);
    buffers.copy_from(BufTy::Ex, start, str);
    buffers.set_offset(BufTy::Ex, 1, start + str.len());
    buffers.set_init(BufTy::Ex, start + str.len());
}

pub(crate) fn add_out_pool(
    ctx: &mut Bibtex<'_, '_>,
    buffers: &mut GlobalBuffer,
    pool: &StringPool,
    str: StrNumber,
) {
    let str = pool.get_str(str);

    while buffers.init(BufTy::Out) + str.len() > buffers.len() {
        buffers.grow_all();
    }

    let out_offset = buffers.init(BufTy::Out);
    buffers.copy_from(BufTy::Out, out_offset, str);
    buffers.set_init(BufTy::Out, out_offset + str.len());

    let mut unbreakable_tail = false;
    while buffers.init(BufTy::Out) > MAX_PRINT_LINE && !unbreakable_tail {
        let end_ptr = buffers.init(BufTy::Out);
        let mut out_offset = MAX_PRINT_LINE;
        let mut break_pt_found = false;

        while LexClass::of(buffers.at(BufTy::Out, out_offset)) != LexClass::Whitespace
            && out_offset >= MIN_PRINT_LINE
        {
            out_offset -= 1;
        }

        if out_offset == MIN_PRINT_LINE - 1 {
            out_offset = MAX_PRINT_LINE + 1;
            while out_offset < end_ptr {
                if LexClass::of(buffers.at(BufTy::Out, out_offset)) != LexClass::Whitespace {
                    out_offset += 1;
                } else {
                    break;
                }
            }

            if out_offset == end_ptr {
                unbreakable_tail = true;
            } else {
                break_pt_found = true;
                while out_offset + 1 < end_ptr {
                    if LexClass::of(buffers.at(BufTy::Out, out_offset + 1)) == LexClass::Whitespace
                    {
                        out_offset += 1;
                    } else {
                        break;
                    }
                }
            }
        } else {
            break_pt_found = true;
        }

        if break_pt_found {
            buffers.set_init(BufTy::Out, out_offset);
            let break_ptr = buffers.init(BufTy::Out) + 1;
            output_bbl_line(ctx, buffers);
            buffers.set_at(BufTy::Out, 0, b' ');
            buffers.set_at(BufTy::Out, 1, b' ');
            let len = end_ptr - break_ptr;
            buffers.copy_within(BufTy::Out, BufTy::Out, break_ptr, 2, len);
            buffers.set_init(BufTy::Out, len + 2);
        }
    }
}

pub(crate) fn check_command_execution(
    ctx: &mut ExecCtx<'_, '_, '_>,
    pool: &mut StringPool,
    hash: &HashData,
    cites: &CiteInfo,
) -> Result<(), BibtexError> {
    if !ctx.lit_stack.is_empty() {
        let msg = format!("ptr={}, stack=\n", ctx.lit_stack.len());
        ctx.write_logs(&msg);
        pop_whole_stack(ctx, pool, hash, cites)?;
        ctx.write_logs("---the literal stack isn't empty");
        bst_ex_warn_print(ctx, pool, cites)?;
    }
    if !pool.is_at(ctx.checkpoint) {
        ctx.write_logs("Nonempty empty string stack");
        print_confusion(ctx);
        return Err(BibtexError::Fatal);
    }
    Ok(())
}

fn add_pool_buf_and_push(
    ctx: &mut ExecCtx<'_, '_, '_>,
    buffers: &mut GlobalBuffer,
    pool: &mut StringPool,
) -> Result<(), BibtexError> {
    buffers.set_offset(BufTy::Ex, 1, buffers.init(BufTy::Ex));
    let str = &buffers.buffer(BufTy::Ex)[0..buffers.init(BufTy::Ex)];
    let val = ExecVal::String(pool.add_string(str));
    ctx.push_stack(val);
    Ok(())
}

fn interp_eq(
    ctx: &mut ExecCtx<'_, '_, '_>,
    pool: &mut StringPool,
    hash: &HashData,
    cites: &CiteInfo,
) -> Result<(), BibtexError> {
    let pop1 = ctx.pop_stack(pool, cites)?;
    let pop2 = ctx.pop_stack(pool, cites)?;

    match (pop1, pop2) {
        (ExecVal::Integer(i1), ExecVal::Integer(i2)) => {
            ctx.push_stack(ExecVal::Integer((i1 == i2) as i64));
        }
        (ExecVal::String(s1), ExecVal::String(s2)) => {
            // TODO: Can we just compare str numbers here?
            ctx.push_stack(ExecVal::Integer(
                (pool.get_str(s1) == pool.get_str(s2)) as i64,
            ));
        }
        _ if pop1.ty() != pop2.ty() => {
            if pop1.ty() != StkType::Illegal && pop2.ty() != StkType::Illegal {
                print_stk_lit(ctx, pool, hash, pop1)?;
                ctx.write_logs(", ");
                print_stk_lit(ctx, pool, hash, pop2)?;
                ctx.write_logs("\n---they aren't the same literal types");
                bst_ex_warn_print(ctx, pool, cites)?;
            }
            ctx.push_stack(ExecVal::Integer(0));
        }
        _ => {
            if pop1.ty() != StkType::Illegal {
                print_stk_lit(ctx, pool, hash, pop1)?;
                ctx.write_logs(", not an integer or a string,");
                bst_ex_warn_print(ctx, pool, cites)?;
            }
            ctx.push_stack(ExecVal::Integer(0))
        }
    }
    Ok(())
}

fn interp_gt(
    ctx: &mut ExecCtx<'_, '_, '_>,
    pool: &mut StringPool,
    hash: &HashData,
    cites: &CiteInfo,
) -> Result<(), BibtexError> {
    let pop1 = ctx.pop_stack(pool, cites)?;
    let pop2 = ctx.pop_stack(pool, cites)?;

    match (pop1, pop2) {
        (ExecVal::Integer(i1), ExecVal::Integer(i2)) => {
            ctx.push_stack(ExecVal::Integer((i2 > i1) as i64));
        }
        (ExecVal::Integer(_), _) => {
            print_wrong_stk_lit(ctx, pool, hash, cites, pop2, StkType::Integer)?;
            ctx.push_stack(ExecVal::Integer(0));
        }
        (_, _) => {
            print_wrong_stk_lit(ctx, pool, hash, cites, pop1, StkType::Integer)?;
            ctx.push_stack(ExecVal::Integer(0));
        }
    }
    Ok(())
}

fn interp_lt(
    ctx: &mut ExecCtx<'_, '_, '_>,
    pool: &mut StringPool,
    hash: &HashData,
    cites: &CiteInfo,
) -> Result<(), BibtexError> {
    let pop1 = ctx.pop_stack(pool, cites)?;
    let pop2 = ctx.pop_stack(pool, cites)?;

    match (pop1, pop2) {
        (ExecVal::Integer(i1), ExecVal::Integer(i2)) => {
            ctx.push_stack(ExecVal::Integer((i2 < i1) as i64));
        }
        (ExecVal::Integer(_), _) => {
            print_wrong_stk_lit(ctx, pool, hash, cites, pop2, StkType::Integer)?;
            ctx.push_stack(ExecVal::Integer(0));
        }
        (_, _) => {
            print_wrong_stk_lit(ctx, pool, hash, cites, pop1, StkType::Integer)?;
            ctx.push_stack(ExecVal::Integer(0));
        }
    }
    Ok(())
}

fn interp_plus(
    ctx: &mut ExecCtx<'_, '_, '_>,
    pool: &mut StringPool,
    hash: &HashData,
    cites: &CiteInfo,
) -> Result<(), BibtexError> {
    let pop1 = ctx.pop_stack(pool, cites)?;
    let pop2 = ctx.pop_stack(pool, cites)?;

    match (pop1, pop2) {
        (ExecVal::Integer(i1), ExecVal::Integer(i2)) => {
            ctx.push_stack(ExecVal::Integer(i2 + i1));
        }
        (ExecVal::Integer(_), _) => {
            print_wrong_stk_lit(ctx, pool, hash, cites, pop2, StkType::Integer)?;
            ctx.push_stack(ExecVal::Integer(0));
        }
        (_, _) => {
            print_wrong_stk_lit(ctx, pool, hash, cites, pop1, StkType::Integer)?;
            ctx.push_stack(ExecVal::Integer(0));
        }
    }
    Ok(())
}

fn interp_minus(
    ctx: &mut ExecCtx<'_, '_, '_>,
    pool: &mut StringPool,
    hash: &HashData,
    cites: &CiteInfo,
) -> Result<(), BibtexError> {
    let pop1 = ctx.pop_stack(pool, cites)?;
    let pop2 = ctx.pop_stack(pool, cites)?;

    match (pop1, pop2) {
        (ExecVal::Integer(i1), ExecVal::Integer(i2)) => {
            ctx.push_stack(ExecVal::Integer(i2 - i1));
        }
        (ExecVal::Integer(_), _) => {
            print_wrong_stk_lit(ctx, pool, hash, cites, pop2, StkType::Integer)?;
            ctx.push_stack(ExecVal::Integer(0));
        }
        (_, _) => {
            print_wrong_stk_lit(ctx, pool, hash, cites, pop1, StkType::Integer)?;
            ctx.push_stack(ExecVal::Integer(0));
        }
    }
    Ok(())
}

fn interp_concat(
    ctx: &mut ExecCtx<'_, '_, '_>,
    pool: &mut StringPool,
    hash: &HashData,
    cites: &CiteInfo,
) -> Result<(), BibtexError> {
    let pop1 = ctx.pop_stack(pool, cites)?;
    let pop2 = ctx.pop_stack(pool, cites)?;

    let (s1, s2) = match (pop1, pop2) {
        (ExecVal::String(s1), ExecVal::String(s2)) => (s1, s2),
        (ExecVal::String(_), _) => {
            print_wrong_stk_lit(ctx, pool, hash, cites, pop2, StkType::String)?;
            ctx.push_stack(ExecVal::String(ctx.s_null));
            return Ok(());
        }
        (_, _) => {
            print_wrong_stk_lit(ctx, pool, hash, cites, pop1, StkType::String)?;
            ctx.push_stack(ExecVal::String(ctx.s_null));
            return Ok(());
        }
    };

    // A string pointer being >= bib_str_ptr means it's a 'scratch string' not yet saved permanently
    // TODO: Add pool API for scratch strings, instead of doing it manually through dangerous manual
    //       implementation of strings

    if ctx.checkpoint.is_before(s2) && ctx.checkpoint.is_before(s1) {
        // Both strings are 'scratch', they must be next to each-other due to external invariants,
        // se we just make one new string covering both
        let new_len = pool.get_str(s1).len() + pool.get_str(s2).len();
        let new = pool.write_str(|cursor| cursor.extend(new_len));
        ctx.push_stack(ExecVal::String(new));
    } else if ctx.checkpoint.is_before(s2) {
        if pool.get_str(s2).is_empty() {
            ctx.push_stack(pop1);
        } else {
            // s2 is scratch, we add s1 to its end and return the new scratch string
            let s2_len = pool.get_str(s2).len();
            let new = pool.write_str(|cursor| {
                cursor.extend(s2_len);
                cursor.append_str(s1);
            });
            ctx.push_stack(ExecVal::String(new));
        }
    } else if ctx.checkpoint.is_before(s1) {
        let str1 = pool.get_str(s1);
        let str2 = pool.get_str(s2);

        if str2.is_empty() {
            // s1 is scratch and s2 is empty - just save s1 and return it
            let s1_len = str1.len();
            let new = pool.write_str(|cursor| cursor.extend(s1_len));
            ctx.push_stack(ExecVal::String(new));
        } else if str1.is_empty() {
            // s1 is empty - just return s2
            ctx.push_stack(pop2);
        } else {
            let s1_len = str1.len();
            let s2_len = str2.len();

            // s1 is scratch and s2 is not - we want to copy s1 forward by the length of s2,
            // then write s2 in where it was, returning the new scratch string
            let new = pool.write_str(|cursor| {
                cursor.extend(s1_len + s2_len);
                let raw = cursor.bytes();
                raw.copy_within(0..s1_len, s2_len);
                cursor.insert_str(s2, 0);
            });
            let val = ExecVal::String(new);
            ctx.push_stack(val);
        }
    } else {
        let str1 = pool.get_str(s1);
        let str2 = pool.get_str(s2);

        if str1.is_empty() {
            ctx.push_stack(pop2);
        } else if str2.is_empty() {
            ctx.push_stack(pop1);
        } else {
            // Neither is scratch or empty - make a new scratch string from the concat of both
            let new = pool.write_str(|cursor| {
                cursor.append_str(s2);
                cursor.append_str(s1);
            });
            let val = ExecVal::String(new);
            ctx.push_stack(val);
        }
    }
    Ok(())
}

fn interp_gets(
    ctx: &mut ExecCtx<'_, '_, '_>,
    pool: &mut StringPool,
    hash: &mut HashData,
    entries: &mut EntryData,
    globals: &mut GlobalData,
    cites: &CiteInfo,
) -> Result<(), BibtexError> {
    let pop1 = ctx.pop_stack(pool, cites)?;
    let pop2 = ctx.pop_stack(pool, cites)?;

    let f1 = match pop1 {
        ExecVal::Function(f1) => f1,
        _ => {
            print_wrong_stk_lit(ctx, pool, hash, cites, pop1, StkType::Function)?;
            return Ok(());
        }
    };

    match &hash.node(f1).extra {
        HashExtra::BstFn(BstFn::IntEntry(_) | BstFn::StrEntry(_)) if !ctx.mess_with_entries => {
            bst_cant_mess_with_entries_print(ctx, pool, cites)?;
        }
        HashExtra::BstFn(BstFn::IntEntry(entry)) => {
            if let ExecVal::Integer(i2) = pop2 {
                entries.set_int(cites.ptr() * entries.num_ent_ints() + *entry, i2)
            } else {
                print_wrong_stk_lit(ctx, pool, hash, cites, pop2, StkType::Integer)?;
            }
        }
        HashExtra::BstFn(BstFn::StrEntry(entry)) => {
            if let ExecVal::String(s2) = pop2 {
                let mut s = pool.get_str(s2);
                if s.len() > ENT_STR_SIZE {
                    bst_1print_string_size_exceeded(ctx);
                    ctx.write_logs(&format!("{ENT_STR_SIZE}, the entry"));
                    bst_2print_string_size_exceeded(ctx, pool, cites)?;
                    s = &s[..ENT_STR_SIZE];
                }
                entries.set_str(cites.ptr() * entries.num_ent_strs() + *entry, s);
            } else {
                print_wrong_stk_lit(ctx, pool, hash, cites, pop2, StkType::Integer)?;
            }
        }
        HashExtra::BstFn(BstFn::IntGlbl(_)) => {
            if let ExecVal::Integer(i2) = pop2 {
                hash.node_mut(f1).extra = HashExtra::BstFn(BstFn::IntGlbl(i2));
            } else {
                print_wrong_stk_lit(ctx, pool, hash, cites, pop2, StkType::Integer)?;
            }
        }
        HashExtra::BstFn(BstFn::StrGlbl(str_ptr)) => {
            if let ExecVal::String(s2) = pop2 {
                if !ctx.checkpoint.is_before(s2) {
                    globals.set_str_ptr(*str_ptr, s2);
                } else {
                    globals.set_str_ptr(*str_ptr, StrNumber::invalid());
                    let mut s = pool.get_str(s2);
                    if s.len() > GLOB_STR_SIZE {
                        bst_1print_string_size_exceeded(ctx);
                        ctx.write_logs(&format!("{GLOB_STR_SIZE}, the global"));
                        bst_2print_string_size_exceeded(ctx, pool, cites)?;
                        s = &s[..GLOB_STR_SIZE];
                    }
                    globals.set_str(*str_ptr, s);
                }
            } else {
                print_wrong_stk_lit(ctx, pool, hash, cites, pop2, StkType::String)?;
            }
        }
        _ => {
            ctx.write_logs("You can't assign to type ");
            print_fn_class(ctx, hash, f1);
            ctx.write_logs(", a nonvariable function class");
            bst_ex_warn_print(ctx, pool, cites)?;
        }
    }
    Ok(())
}

fn interp_add_period(
    ctx: &mut ExecCtx<'_, '_, '_>,
    pool: &mut StringPool,
    hash: &HashData,
    cites: &CiteInfo,
) -> Result<(), BibtexError> {
    let pop1 = ctx.pop_stack(pool, cites)?;

    let s1 = match pop1 {
        ExecVal::String(s1) => s1,
        _ => {
            print_wrong_stk_lit(ctx, pool, hash, cites, pop1, StkType::String)?;
            ctx.push_stack(ExecVal::String(ctx.s_null));
            return Ok(());
        }
    };

    let str = pool.get_str(s1);

    if str.is_empty() {
        ctx.push_stack(ExecVal::String(ctx.s_null));
        return Ok(());
    }

    let pos = str.iter().copied().rposition(|c| c != b'}').unwrap_or(0);

    match str[pos] {
        b'.' | b'?' | b'!' => {
            // If scratch, save
            if ctx.checkpoint.is_before(s1) {
                let s1_len = pool.get_str(s1).len();
                let new = pool.write_str(|cursor| cursor.extend(s1_len));
                ctx.push_stack(ExecVal::String(new));
            } else {
                ctx.push_stack(pop1);
            }
        }
        _ => {
            let is_bst_str = ctx.checkpoint.is_before(s1);
            let s1_len = pool.get_str(s1).len();
            let new = pool.write_str(|cursor| {
                if is_bst_str {
                    cursor.extend(s1_len);
                } else {
                    cursor.append_str(s1);
                }
                cursor.append(b'.');
            });
            let val = ExecVal::String(new);
            ctx.push_stack(val);
        }
    }
    Ok(())
}

fn interp_change_case(
    ctx: &mut ExecCtx<'_, '_, '_>,
    pool: &mut StringPool,
    cites: &CiteInfo,
    hash: &HashData,
) -> Result<(), BibtexError> {
    #[derive(PartialEq)]
    enum ConvTy {
        TitleLower,
        AllLower,
        AllUpper,
        Bad,
    }

    let pop1 = ctx.pop_stack(pool, cites)?;
    let pop2 = ctx.pop_stack(pool, cites)?;

    match (pop1, pop2) {
        (ExecVal::String(s1), ExecVal::String(s2)) => {
            let mut prev_colon = false;

            let str1 = pool.get_str(s1);
            let conv_ty = if str1.len() == 1 {
                match str1[0] {
                    b't' | b'T' => ConvTy::TitleLower,
                    b'l' | b'L' => ConvTy::AllLower,
                    b'u' | b'U' => ConvTy::AllUpper,
                    _ => ConvTy::Bad,
                }
            } else {
                ConvTy::Bad
            };

            if conv_ty == ConvTy::Bad {
                print_a_pool_str(ctx, s1, pool)?;
                ctx.write_logs(" is an illegal case-conversion string");
                bst_ex_warn_print(ctx, pool, cites)?;
            }

            let mut scratch = Vec::from(pool.get_str(s2));

            let mut brace_level = 0;
            let mut idx = 0;
            while idx < scratch.len() {
                if scratch[idx] == b'{' {
                    brace_level += 1;
                    if !(brace_level != 1
                        || idx + 4 > scratch.len()
                        || scratch[idx + 1] != b'\\'
                        || (conv_ty == ConvTy::TitleLower
                            && (idx == 0
                                || (prev_colon
                                    && LexClass::of(scratch[idx - 1]) == LexClass::Whitespace))))
                    {
                        idx += 1;

                        while idx < scratch.len() && brace_level > 0 {
                            idx += 1;
                            let old_idx = idx;
                            while idx < scratch.len()
                                && LexClass::of(scratch[idx]) == LexClass::Alpha
                            {
                                idx += 1;
                            }

                            let res =
                                hash.lookup_str(pool, &scratch[old_idx..idx], StrIlk::ControlSeq);

                            if res.exists {
                                let HashExtra::ControlSeq(seq) = hash.node(res.loc).extra else {
                                    panic!("ControlSeq lookup didn't have ControlSeq extra");
                                };
                                match conv_ty {
                                    ConvTy::TitleLower | ConvTy::AllLower => match seq {
                                        ControlSeq::UpperOE
                                        | ControlSeq::UpperAE
                                        | ControlSeq::UpperAA
                                        | ControlSeq::UpperO
                                        | ControlSeq::UpperL => {
                                            scratch[old_idx..idx].make_ascii_lowercase()
                                        }
                                        _ => (),
                                    },
                                    ConvTy::AllUpper => match seq {
                                        ControlSeq::LowerOE
                                        | ControlSeq::LowerAE
                                        | ControlSeq::LowerAA
                                        | ControlSeq::LowerO
                                        | ControlSeq::LowerL => {
                                            scratch[old_idx..idx].make_ascii_uppercase()
                                        }
                                        ControlSeq::LowerI
                                        | ControlSeq::LowerJ
                                        | ControlSeq::LowerSS => {
                                            scratch[old_idx..idx].make_ascii_uppercase();
                                            scratch.copy_within(old_idx..idx, old_idx - 1);
                                            let old_idx = idx - 1;
                                            while idx < scratch.len()
                                                && LexClass::of(scratch[idx])
                                                    == LexClass::Whitespace
                                            {
                                                idx += 1;
                                            }
                                            scratch.copy_within(idx.., old_idx);
                                            scratch.truncate(scratch.len() - idx + old_idx);
                                            idx = old_idx;
                                        }
                                        _ => (),
                                    },
                                    ConvTy::Bad => (),
                                }
                            }

                            let old_idx = idx;
                            while idx < scratch.len() && brace_level > 0 && scratch[idx] != b'\\' {
                                match scratch[idx] {
                                    b'{' => brace_level += 1,
                                    b'}' => brace_level -= 1,
                                    _ => (),
                                }
                                idx += 1;
                            }

                            match conv_ty {
                                ConvTy::TitleLower | ConvTy::AllLower => {
                                    scratch[old_idx..idx].make_ascii_lowercase()
                                }
                                ConvTy::AllUpper => scratch[old_idx..idx].make_ascii_uppercase(),
                                ConvTy::Bad => (),
                            }
                        }
                        idx -= 1;
                    }

                    prev_colon = false;
                } else if scratch[idx] == b'}' {
                    decr_brace_level(ctx, pool, cites, s2, &mut brace_level)?;
                    prev_colon = false;
                } else if brace_level == 0 {
                    match conv_ty {
                        ConvTy::TitleLower => {
                            if idx != 0
                                && !(prev_colon
                                    && LexClass::of(scratch[idx - 1]) == LexClass::Whitespace)
                            {
                                scratch[idx].make_ascii_lowercase()
                            }

                            if scratch[idx] == b':' {
                                prev_colon = true;
                            } else if LexClass::of(scratch[idx]) != LexClass::Whitespace {
                                prev_colon = false;
                            }
                        }
                        ConvTy::AllLower => scratch[idx].make_ascii_lowercase(),
                        ConvTy::AllUpper => scratch[idx].make_ascii_uppercase(),
                        ConvTy::Bad => (),
                    }
                }
                idx += 1;
            }
            check_brace_level(ctx, pool, cites, s2, brace_level)?;
            let val = ExecVal::String(pool.add_string(&scratch));
            ctx.push_stack(val);
        }
        (ExecVal::String(_), _) => {
            print_wrong_stk_lit(ctx, pool, hash, cites, pop2, StkType::String)?;
            ctx.push_stack(ExecVal::String(ctx.s_null));
        }
        (_, _) => {
            print_wrong_stk_lit(ctx, pool, hash, cites, pop1, StkType::String)?;
            ctx.push_stack(ExecVal::String(ctx.s_null));
        }
    }
    Ok(())
}

fn interp_chr_to_int(
    ctx: &mut ExecCtx<'_, '_, '_>,
    pool: &mut StringPool,
    hash: &HashData,
    cites: &CiteInfo,
) -> Result<(), BibtexError> {
    let pop1 = ctx.pop_stack(pool, cites)?;
    match pop1 {
        ExecVal::String(s1) => {
            let str = pool.get_str(s1);
            if str.len() != 1 {
                ctx.write_logs("\"");
                print_a_pool_str(ctx, s1, pool)?;
                ctx.write_logs("\" isn't a single character");
                bst_ex_warn_print(ctx, pool, cites)?;
                ctx.push_stack(ExecVal::Integer(0));
            } else {
                ctx.push_stack(ExecVal::Integer(str[0] as i64))
            }
        }
        _ => {
            print_wrong_stk_lit(ctx, pool, hash, cites, pop1, StkType::String)?;
            ctx.push_stack(ExecVal::Integer(0));
        }
    }
    Ok(())
}

fn interp_cite(
    ctx: &mut ExecCtx<'_, '_, '_>,
    pool: &StringPool,
    cites: &CiteInfo,
) -> Result<(), BibtexError> {
    if !ctx.mess_with_entries {
        bst_cant_mess_with_entries_print(ctx, pool, cites)?;
    } else {
        ctx.push_stack(ExecVal::String(cites.get_cite(cites.ptr())))
    }
    Ok(())
}

fn interp_dup(
    ctx: &mut ExecCtx<'_, '_, '_>,
    pool: &mut StringPool,
    cites: &CiteInfo,
) -> Result<(), BibtexError> {
    let pop1 = ctx.pop_stack(pool, cites)?;
    match pop1 {
        ExecVal::String(s1) => {
            ctx.push_stack(pop1);
            if !ctx.checkpoint.is_before(s1) {
                ctx.push_stack(pop1);
            } else {
                let str_len = pool.get_str(s1).len();
                let _ = pool.write_str(|cursor| {
                    cursor.extend(str_len);
                });
                let new = pool.write_str(|cursor| {
                    cursor.append_str(s1);
                });
                let val = ExecVal::String(new);
                ctx.push_stack(val);
            }
        }
        _ => {
            ctx.push_stack(pop1);
            ctx.push_stack(pop1);
        }
    }
    Ok(())
}

fn interp_empty(
    ctx: &mut ExecCtx<'_, '_, '_>,
    pool: &mut StringPool,
    hash: &HashData,
    cites: &CiteInfo,
) -> Result<(), BibtexError> {
    let pop1 = ctx.pop_stack(pool, cites)?;
    match pop1 {
        ExecVal::String(s1) => {
            let str = pool.get_str(s1);
            let res = str.iter().all(|c| LexClass::of(*c) == LexClass::Whitespace);
            ctx.push_stack(ExecVal::Integer(res as i64));
        }
        ExecVal::Missing(_) => {
            ctx.push_stack(ExecVal::Integer(1));
        }
        ExecVal::Illegal => {
            ctx.push_stack(ExecVal::Integer(0));
        }
        _ => {
            print_stk_lit(ctx, pool, hash, pop1)?;
            ctx.write_logs(", not a string or missing field,");
            bst_ex_warn_print(ctx, pool, cites)?;
            ctx.push_stack(ExecVal::Integer(0));
        }
    }
    Ok(())
}

fn interp_format_name(
    ctx: &mut ExecCtx<'_, '_, '_>,
    pool: &mut StringPool,
    buffers: &mut GlobalBuffer,
    cites: &CiteInfo,
    hash: &HashData,
) -> Result<(), BibtexError> {
    let pop1 = ctx.pop_stack(pool, cites)?;
    let pop2 = ctx.pop_stack(pool, cites)?;
    let pop3 = ctx.pop_stack(pool, cites)?;

    let (s1, i2, s3) = match (pop1, pop2, pop3) {
        (ExecVal::String(s1), ExecVal::Integer(i2), ExecVal::String(s3)) => (s1, i2, s3),
        (ExecVal::String(_), ExecVal::Integer(_), _) => {
            print_wrong_stk_lit(ctx, pool, hash, cites, pop3, StkType::String)?;
            ctx.push_stack(ExecVal::String(ctx.s_null));
            return Ok(());
        }
        (ExecVal::String(_), _, _) => {
            print_wrong_stk_lit(ctx, pool, hash, cites, pop2, StkType::Integer)?;
            ctx.push_stack(ExecVal::String(ctx.s_null));
            return Ok(());
        }
        (_, _, _) => {
            print_wrong_stk_lit(ctx, pool, hash, cites, pop1, StkType::String)?;
            ctx.push_stack(ExecVal::String(ctx.s_null));
            return Ok(());
        }
    };

    let mut brace_level = 0;
    let mut xptr = 0;
    buffers.set_init(BufTy::Ex, 0);
    add_buf_pool(pool, buffers, s3);
    buffers.set_offset(BufTy::Ex, 1, 0);

    let mut num_names = 0;
    while num_names < i2 && buffers.offset(BufTy::Ex, 1) < buffers.init(BufTy::Ex) {
        num_names += 1;
        xptr = buffers.offset(BufTy::Ex, 1);
        name_scan_for_and(ctx, pool, buffers, cites, s3, &mut brace_level)?;
    }

    if buffers.offset(BufTy::Ex, 1) < buffers.init(BufTy::Ex) {
        buffers.set_offset(BufTy::Ex, 1, buffers.offset(BufTy::Ex, 1) - 4);
    }

    if num_names < i2 {
        if i2 == 1 {
            ctx.write_logs("There is no name in \"");
        } else {
            ctx.write_logs(&format!("There aren't {i2} names in \""));
        }
        print_a_pool_str(ctx, s3, pool)?;
        ctx.write_logs("\"");
        bst_ex_warn_print(ctx, pool, cites)?;
    }

    while buffers.offset(BufTy::Ex, 1) > xptr {
        match LexClass::of(buffers.at(BufTy::Ex, buffers.offset(BufTy::Ex, 1) - 1)) {
            LexClass::Whitespace | LexClass::Sep => {
                buffers.set_offset(BufTy::Ex, 1, buffers.offset(BufTy::Ex, 1) - 1);
            }
            _ => {
                if buffers.at(BufTy::Ex, buffers.offset(BufTy::Ex, 1) - 1) == b',' {
                    ctx.write_logs(&format!("Name {i2} in \""));
                    print_a_pool_str(ctx, s3, pool)?;
                    ctx.write_logs("\" has a comma at the end");
                    bst_ex_warn_print(ctx, pool, cites)?;
                    buffers.set_offset(BufTy::Ex, 1, buffers.offset(BufTy::Ex, 1) - 1);
                } else {
                    break;
                }
            }
        }
    }

    enum Commas {
        None,
        One(BufPointer),
        Two(BufPointer, BufPointer),
    }

    let mut num_tokens = 0;
    let mut commas = Commas::None;
    let mut name_ptr = 0;
    let mut token_starting = true;

    while xptr < buffers.offset(BufTy::Ex, 1) {
        match buffers.at(BufTy::Ex, xptr) {
            b',' => {
                match commas {
                    Commas::None => {
                        commas = Commas::One(num_tokens);
                        buffers.set_at(BufTy::NameSep, num_tokens, b',');
                    }
                    Commas::One(first) => {
                        commas = Commas::Two(first, num_tokens);
                        buffers.set_at(BufTy::NameSep, num_tokens, b',');
                    }
                    Commas::Two(_, _) => {
                        ctx.write_logs(&format!("Too many commas in name {i2} of \""));
                        print_a_pool_str(ctx, s3, pool)?;
                        ctx.write_logs("\"");
                        bst_ex_warn_print(ctx, pool, cites)?;
                    }
                }
                xptr += 1;
                token_starting = true;
            }
            b'{' => {
                brace_level += 1;
                if token_starting {
                    buffers.set_name_tok(num_tokens, name_ptr);
                    num_tokens += 1;
                }
                buffers.set_at(BufTy::Sv, name_ptr, buffers.at(BufTy::Ex, xptr));
                name_ptr += 1;
                xptr += 1;
                while brace_level > 0 && xptr < buffers.offset(BufTy::Ex, 1) {
                    match buffers.at(BufTy::Ex, xptr) {
                        b'{' => brace_level += 1,
                        b'}' => brace_level -= 1,
                        _ => (),
                    }
                    buffers.set_at(BufTy::Sv, name_ptr, buffers.at(BufTy::Ex, xptr));
                    name_ptr += 1;
                    xptr += 1;
                }
                token_starting = false;
            }
            b'}' => {
                if token_starting {
                    buffers.set_name_tok(num_tokens, name_ptr);
                    num_tokens += 1;
                }

                ctx.write_logs(&format!("Name {i2} of \""));
                print_a_pool_str(ctx, s3, pool)?;
                ctx.write_logs("\" isn't brace balanced");
                bst_ex_warn_print(ctx, pool, cites)?;
                xptr += 1;
                token_starting = false;
            }
            _ => match LexClass::of(buffers.at(BufTy::Ex, xptr)) {
                LexClass::Whitespace => {
                    if !token_starting {
                        buffers.set_at(BufTy::NameSep, num_tokens, b' ');
                    }
                    xptr += 1;
                    token_starting = true;
                }
                LexClass::Sep => {
                    if !token_starting {
                        buffers.set_at(BufTy::NameSep, num_tokens, buffers.at(BufTy::Ex, xptr));
                    }
                    xptr += 1;
                    token_starting = true;
                }
                _ => {
                    if token_starting {
                        buffers.set_name_tok(num_tokens, name_ptr);
                        num_tokens += 1;
                    }
                    buffers.set_at(BufTy::Sv, name_ptr, buffers.at(BufTy::Ex, xptr));
                    name_ptr += 1;
                    xptr += 1;
                    token_starting = false;
                }
            },
        }
    }

    buffers.set_name_tok(num_tokens, name_ptr);

    let mut first_start = 0;
    let first_end;
    let last_end;
    let mut von_start = 0;
    let mut von_end = 0;
    let jr_end;
    let mut name_ptr2 = 0;

    match commas {
        Commas::None => {
            last_end = num_tokens;
            jr_end = last_end;

            let mut second_loop = true;
            while von_start < last_end - 1 {
                name_ptr = buffers.name_tok(von_start);
                name_ptr2 = buffers.name_tok(von_start + 1);
                if von_token_found(buffers, hash, pool, &mut name_ptr, name_ptr2) {
                    von_name_ends_and_last_name_starts_stuff(
                        buffers,
                        hash,
                        pool,
                        last_end,
                        von_start,
                        &mut von_end,
                        &mut name_ptr,
                        &mut name_ptr2,
                    );
                    second_loop = false;
                    break;
                }
                von_start += 1;
            }

            if second_loop {
                while von_start > 0 {
                    if LexClass::of(buffers.at(BufTy::NameSep, von_start)) != LexClass::Sep
                        || buffers.at(BufTy::NameSep, von_start) == b'~'
                    {
                        break;
                    }
                    von_start -= 1;
                }
                von_end = von_start;
            }
            first_end = von_start;
        }
        Commas::One(comma) => {
            last_end = comma;
            jr_end = last_end;
            first_start = jr_end;
            first_end = num_tokens;
            von_name_ends_and_last_name_starts_stuff(
                buffers,
                hash,
                pool,
                last_end,
                von_start,
                &mut von_end,
                &mut name_ptr,
                &mut name_ptr2,
            );
        }
        Commas::Two(comma1, comma2) => {
            last_end = comma1;
            jr_end = comma2;
            first_start = jr_end;
            first_end = num_tokens;
            von_name_ends_and_last_name_starts_stuff(
                buffers,
                hash,
                pool,
                last_end,
                von_start,
                &mut von_end,
                &mut name_ptr,
                &mut name_ptr2,
            );
        }
    }

    buffers.set_init(BufTy::Ex, 0);
    add_buf_pool(pool, buffers, s1);
    figure_out_the_formatted_name(
        ctx,
        buffers,
        pool,
        cites,
        s1,
        first_start,
        first_end,
        last_end,
        von_start,
        von_end,
        &mut name_ptr,
        &mut name_ptr2,
        jr_end,
        &mut brace_level,
    )?;
    add_pool_buf_and_push(ctx, buffers, pool)?;

    Ok(())
}

fn interp_int_to_chr(
    ctx: &mut ExecCtx<'_, '_, '_>,
    pool: &mut StringPool,
    hash: &HashData,
    cites: &CiteInfo,
) -> Result<(), BibtexError> {
    let pop1 = ctx.pop_stack(pool, cites)?;
    let i1 = match pop1 {
        ExecVal::Integer(i1) => i1,
        _ => {
            print_wrong_stk_lit(ctx, pool, hash, cites, pop1, StkType::Integer)?;
            ctx.push_stack(ExecVal::String(ctx.s_null));
            return Ok(());
        }
    };

    if !(0..=127).contains(&i1) {
        ctx.write_logs(&format!("{i1} isn't valid ASCII"));
        bst_ex_warn_print(ctx, pool, cites)?;
        ctx.push_stack(ExecVal::String(ctx.s_null));
    } else {
        let val = ExecVal::String(pool.add_string(&[i1 as u8]));
        ctx.push_stack(val);
    }
    Ok(())
}

fn interp_int_to_str(
    ctx: &mut ExecCtx<'_, '_, '_>,
    pool: &mut StringPool,
    hash: &HashData,
    cites: &CiteInfo,
) -> Result<(), BibtexError> {
    let pop1 = ctx.pop_stack(pool, cites)?;
    let i1 = match pop1 {
        ExecVal::Integer(i1) => i1,
        _ => {
            print_wrong_stk_lit(ctx, pool, hash, cites, pop1, StkType::Integer)?;
            ctx.push_stack(ExecVal::String(ctx.s_null));
            return Ok(());
        }
    };

    let scratch = i1.to_string();
    let val = ExecVal::String(pool.add_string(scratch.as_bytes()));
    ctx.push_stack(val);
    Ok(())
}

fn interp_missing(
    ctx: &mut ExecCtx<'_, '_, '_>,
    pool: &mut StringPool,
    hash: &HashData,
    cites: &CiteInfo,
) -> Result<(), BibtexError> {
    let pop1 = ctx.pop_stack(pool, cites)?;
    if !ctx.mess_with_entries {
        bst_cant_mess_with_entries_print(ctx, pool, cites)?;
        return Ok(());
    }
    match pop1 {
        ExecVal::String(_) => {
            ctx.push_stack(ExecVal::Integer(0));
        }
        ExecVal::Missing(_) => {
            ctx.push_stack(ExecVal::Integer(1));
        }
        ExecVal::Illegal => {
            ctx.push_stack(ExecVal::Integer(0));
        }
        _ => {
            print_stk_lit(ctx, pool, hash, pop1)?;
            ctx.write_logs(", not a string or missing field,");
            bst_ex_warn_print(ctx, pool, cites)?;
            ctx.push_stack(ExecVal::Integer(0));
        }
    }
    Ok(())
}

fn interp_num_names(
    ctx: &mut ExecCtx<'_, '_, '_>,
    pool: &mut StringPool,
    buffers: &mut GlobalBuffer,
    hash: &HashData,
    cites: &CiteInfo,
) -> Result<(), BibtexError> {
    let pop1 = ctx.pop_stack(pool, cites)?;
    match pop1 {
        ExecVal::String(s1) => {
            buffers.set_init(BufTy::Ex, 0);
            add_buf_pool(pool, buffers, s1);
            buffers.set_offset(BufTy::Ex, 1, 0);
            let mut num_names = 0;
            while buffers.offset(BufTy::Ex, 1) < buffers.init(BufTy::Ex) {
                let mut brace_level = 0;
                name_scan_for_and(ctx, pool, buffers, cites, s1, &mut brace_level)?;
                num_names += 1;
            }
            ctx.push_stack(ExecVal::Integer(num_names))
        }
        _ => {
            print_wrong_stk_lit(ctx, pool, hash, cites, pop1, StkType::String)?;
            ctx.push_stack(ExecVal::Integer(0));
        }
    }
    Ok(())
}

fn interp_preamble(
    ctx: &mut ExecCtx<'_, '_, '_>,
    pool: &mut StringPool,
    bibs: &mut BibData,
) -> Result<(), BibtexError> {
    let mut out = Vec::with_capacity(bibs.preamble_len() * 32);
    for s in bibs.preamble() {
        out.extend(pool.get_str(*s));
    }
    let s = pool.add_string(&out);
    ctx.push_stack(ExecVal::String(s));
    Ok(())
}

fn interp_purify(
    ctx: &mut ExecCtx<'_, '_, '_>,
    pool: &mut StringPool,
    hash: &HashData,
    cites: &CiteInfo,
) -> Result<(), BibtexError> {
    let pop1 = ctx.pop_stack(pool, cites)?;
    let s1 = match pop1 {
        ExecVal::String(s1) => s1,
        _ => {
            print_wrong_stk_lit(ctx, pool, hash, cites, pop1, StkType::String)?;
            ctx.push_stack(ExecVal::String(ctx.s_null));
            return Ok(());
        }
    };

    let mut scratch = Vec::from(pool.get_str(s1));
    let mut idx = 0;
    let mut brace_level: i32 = 0;
    let mut write_idx = 0;

    while idx < scratch.len() {
        match LexClass::of(scratch[idx]) {
            LexClass::Whitespace | LexClass::Sep => {
                scratch[write_idx] = b' ';
                write_idx += 1;
            }
            LexClass::Alpha | LexClass::Numeric => {
                scratch[write_idx] = scratch[idx];
                write_idx += 1;
            }
            _ => match scratch[idx] {
                b'{' => {
                    brace_level += 1;
                    if brace_level == 1 && idx + 1 < scratch.len() && scratch[idx + 1] == b'\\' {
                        idx += 1;
                        while idx < scratch.len() && brace_level > 0 {
                            idx += 1;
                            let old_idx = idx;
                            while idx < scratch.len()
                                && LexClass::of(scratch[idx]) == LexClass::Alpha
                            {
                                idx += 1;
                            }

                            let res =
                                hash.lookup_str(pool, &scratch[old_idx..idx], StrIlk::ControlSeq);
                            if res.exists {
                                let HashExtra::ControlSeq(seq) = hash.node(res.loc).extra else {
                                    panic!("ControlSeq lookup didn't have ControlSeq extra");
                                };
                                scratch[write_idx] = scratch[old_idx];
                                write_idx += 1;
                                match seq {
                                    ControlSeq::LowerOE
                                    | ControlSeq::UpperOE
                                    | ControlSeq::LowerAE
                                    | ControlSeq::UpperAE
                                    | ControlSeq::LowerSS => {
                                        scratch[write_idx] = scratch[old_idx + 1];
                                        write_idx += 1;
                                    }
                                    _ => (),
                                }
                            }
                            while idx < scratch.len() && brace_level > 0 && scratch[idx] != b'\\' {
                                match LexClass::of(scratch[idx]) {
                                    LexClass::Alpha | LexClass::Numeric => {
                                        scratch[write_idx] = scratch[idx];
                                        write_idx += 1;
                                    }
                                    _ => match scratch[idx] {
                                        b'{' => brace_level += 1,
                                        b'}' => brace_level -= 1,
                                        _ => (),
                                    },
                                }
                                idx += 1;
                            }
                        }
                        idx -= 1;
                    }
                }
                b'}' => {
                    brace_level = brace_level.saturating_sub(1);
                }
                _ => (),
            },
        }
        idx += 1;
    }

    scratch.truncate(write_idx);
    let out = pool.add_string(&scratch);
    ctx.push_stack(ExecVal::String(out));

    Ok(())
}

fn interp_quote(ctx: &mut ExecCtx<'_, '_, '_>, pool: &mut StringPool) -> Result<(), BibtexError> {
    let s = pool.add_string(b"\"");
    ctx.push_stack(ExecVal::String(s));
    Ok(())
}

#[derive(Copy, Clone)]
struct SLRange {
    start: isize,
    len: usize,
}

impl<T> Index<SLRange> for [T] {
    type Output = [T];

    fn index(&self, index: SLRange) -> &Self::Output {
        let len = usize::min(self.len() + 1 - index.start.unsigned_abs(), index.len);

        match index.start {
            ..=-1 => {
                let start = index.start.unsigned_abs() - 1;
                &self[self.len() - start - len..self.len() - start]
            }
            1.. => {
                let start = index.start as usize - 1;
                &self[start..start + len]
            }
            _ => &[],
        }
    }
}

fn interp_substr(
    ctx: &mut ExecCtx<'_, '_, '_>,
    pool: &mut StringPool,
    hash: &HashData,
    cites: &CiteInfo,
) -> Result<(), BibtexError> {
    let pop1 = ctx.pop_stack(pool, cites)?;
    let pop2 = ctx.pop_stack(pool, cites)?;
    let pop3 = ctx.pop_stack(pool, cites)?;

    let (len, start, s3) = match (pop1, pop2, pop3) {
        (ExecVal::Integer(i1), ExecVal::Integer(i2), ExecVal::String(s3)) => (i1, i2, s3),
        (ExecVal::Integer(_), ExecVal::Integer(_), _) => {
            print_wrong_stk_lit(ctx, pool, hash, cites, pop3, StkType::String)?;
            ctx.push_stack(ExecVal::String(ctx.s_null));
            return Ok(());
        }
        (ExecVal::Integer(_), _, _) => {
            print_wrong_stk_lit(ctx, pool, hash, cites, pop2, StkType::Integer)?;
            ctx.push_stack(ExecVal::String(ctx.s_null));
            return Ok(());
        }
        (_, _, _) => {
            print_wrong_stk_lit(ctx, pool, hash, cites, pop1, StkType::Integer)?;
            ctx.push_stack(ExecVal::String(ctx.s_null));
            return Ok(());
        }
    };

    let str = pool.get_str(s3);

    if len <= 0 || start == 0 || start.unsigned_abs() as usize > str.len() {
        ctx.push_stack(ExecVal::String(ctx.s_null));
        return Ok(());
    }

    let len = len as usize;
    let start = start as isize;

    if len >= str.len() && (start == 1 || start == -1) {
        if ctx.checkpoint.is_before(s3) {
            let str_len = pool.get_str(s3).len();
            let _ = pool.write_str(|cursor| cursor.extend(str_len));
        }
        ctx.push_stack(pop3);
        return Ok(());
    }

    if start == 1 && ctx.checkpoint.is_before(s3) {
        let new = pool.write_str(|cursor| {
            cursor.extend(len);
        });
        ctx.push_stack(ExecVal::String(new));
        return Ok(());
    }

    // TODO: Remove this intermediate allocation, currently can't pass a `&str` from a StringPool
    //       to that StringPool.
    let new_str = Vec::from(&str[SLRange { start, len }]);
    let out = pool.add_string(&new_str);
    ctx.push_stack(ExecVal::String(out));

    Ok(())
}

fn interp_swap(
    ctx: &mut ExecCtx<'_, '_, '_>,
    pool: &mut StringPool,
    cites: &CiteInfo,
) -> Result<(), BibtexError> {
    let pop1 = ctx.pop_stack(pool, cites)?;
    let pop2 = ctx.pop_stack(pool, cites)?;

    match (pop1, pop2) {
        (ExecVal::String(s1), ExecVal::String(s2))
            if ctx.checkpoint.is_before(s1) && ctx.checkpoint.is_before(s2) =>
        {
            let tmp = Vec::from(pool.get_str(s2));
            let new = pool.write_str(|cursor| {
                cursor.append_str(s1);
            });
            let val = ExecVal::String(new);
            ctx.push_stack(val);
            let val = ExecVal::String(pool.add_string(&tmp));
            ctx.push_stack(val);
            return Ok(());
        }
        (ExecVal::String(s), _) | (_, ExecVal::String(s)) if ctx.checkpoint.is_before(s) => {
            let str_len = pool.get_str(s).len();
            let _ = pool.write_str(|cursor| {
                cursor.extend(str_len);
            });
        }
        (_, _) => (),
    }
    ctx.push_stack(pop1);
    ctx.push_stack(pop2);
    Ok(())
}

fn interp_text_len(
    ctx: &mut ExecCtx<'_, '_, '_>,
    pool: &mut StringPool,
    hash: &HashData,
    cites: &CiteInfo,
) -> Result<(), BibtexError> {
    let pop1 = ctx.pop_stack(pool, cites)?;

    let s1 = match pop1 {
        ExecVal::String(s1) => s1,
        _ => {
            print_wrong_stk_lit(ctx, pool, hash, cites, pop1, StkType::String)?;
            ctx.push_stack(ExecVal::String(ctx.s_null));
            return Ok(());
        }
    };

    let str = pool.get_str(s1);
    let mut idx = 0;
    let mut brace_level: i32 = 0;
    let mut num_chars = 0;
    while idx < str.len() {
        idx += 1;
        match str[idx - 1] {
            b'{' => {
                brace_level += 1;
                if brace_level == 1 && idx < str.len() && str[idx] == b'\\' {
                    idx += 1;
                    while idx < str.len() && brace_level > 0 {
                        match str[idx] {
                            b'{' => brace_level += 1,
                            b'}' => brace_level -= 1,
                            _ => (),
                        }
                        idx += 1;
                        num_chars += 1;
                    }
                }
            }
            b'}' => {
                brace_level = brace_level.saturating_sub(1);
            }
            _ => num_chars += 1,
        }
    }

    ctx.push_stack(ExecVal::Integer(num_chars));
    Ok(())
}

fn interp_text_prefix(
    ctx: &mut ExecCtx<'_, '_, '_>,
    pool: &mut StringPool,
    hash: &HashData,
    cites: &CiteInfo,
) -> Result<(), BibtexError> {
    let pop1 = ctx.pop_stack(pool, cites)?;
    let pop2 = ctx.pop_stack(pool, cites)?;

    let (i1, s2) = match (pop1, pop2) {
        (ExecVal::Integer(i1), ExecVal::String(s2)) => (i1, s2),
        (ExecVal::Integer(_), _) => {
            print_wrong_stk_lit(ctx, pool, hash, cites, pop2, StkType::String)?;
            ctx.push_stack(ExecVal::String(ctx.s_null));
            return Ok(());
        }
        (_, _) => {
            print_wrong_stk_lit(ctx, pool, hash, cites, pop1, StkType::Integer)?;
            ctx.push_stack(ExecVal::String(ctx.s_null));
            return Ok(());
        }
    };

    if i1 <= 0 {
        ctx.push_stack(ExecVal::String(ctx.s_null));
        return Ok(());
    }

    let mut brace_level: usize = 0;
    let str = pool.get_str(s2);
    let mut num_chars = 0;
    let mut idx = 0;
    while idx < str.len() && num_chars < i1 {
        idx += 1;
        match str[idx - 1] {
            b'{' => {
                brace_level += 1;
                if brace_level == 1 && idx < str.len() && str[idx] == b'\\' {
                    idx += 1;
                    while idx < str.len() && brace_level > 0 {
                        match str[idx] {
                            b'{' => brace_level += 1,
                            b'}' => brace_level -= 1,
                            _ => (),
                        }
                        num_chars += 1;
                    }
                }
            }
            b'}' => {
                brace_level = brace_level.saturating_sub(1);
            }
            _ => num_chars += 1,
        }
    }

    let is_before = ctx.checkpoint.is_before(s2);
    let new = pool.write_str(|cursor| {
        if is_before {
            cursor.extend(idx)
        } else {
            cursor.append_substr(s2, 0..idx)
        }
        for _ in 0..brace_level {
            cursor.append(b'}');
        }
    });

    let val = ExecVal::String(new);
    ctx.push_stack(val);
    Ok(())
}

fn interp_ty(
    ctx: &mut ExecCtx<'_, '_, '_>,
    pool: &StringPool,
    hash: &HashData,
    cites: &CiteInfo,
) -> Result<(), BibtexError> {
    if !ctx.mess_with_entries {
        bst_cant_mess_with_entries_print(ctx, pool, cites)?;
        return Ok(());
    }

    let ty = cites.get_type(cites.ptr());
    let s = if ty == HashData::undefined() || ty == 0 {
        ctx.s_null
    } else {
        hash.text(ty)
    };
    ctx.push_stack(ExecVal::String(s));
    Ok(())
}

fn interp_warning(
    ctx: &mut ExecCtx<'_, '_, '_>,
    pool: &mut StringPool,
    hash: &HashData,
    cites: &CiteInfo,
) -> Result<(), BibtexError> {
    let pop1 = ctx.pop_stack(pool, cites)?;
    match pop1 {
        ExecVal::String(_) => {
            ctx.write_logs("Warning--");
            print_lit(ctx, pool, hash, pop1)?;
            ctx.mark_warning();
        }
        _ => print_wrong_stk_lit(ctx, pool, hash, cites, pop1, StkType::String)?,
    }
    Ok(())
}

fn interp_width(
    ctx: &mut ExecCtx<'_, '_, '_>,
    pool: &mut StringPool,
    hash: &HashData,
    cites: &CiteInfo,
) -> Result<(), BibtexError> {
    let pop1 = ctx.pop_stack(pool, cites)?;

    let s1 = match pop1 {
        ExecVal::String(s1) => s1,
        _ => {
            print_wrong_stk_lit(ctx, pool, hash, cites, pop1, StkType::String)?;
            ctx.push_stack(ExecVal::Integer(0));
            return Ok(());
        }
    };

    let str = pool.get_str(s1);

    let mut string_width = 0;
    let mut brace_level = 0;
    let mut idx = 0;

    while idx < str.len() {
        match str[idx] {
            b'{' => {
                brace_level += 1;
                if brace_level == 1 && idx + 1 < str.len() && str[idx + 1] == b'\\' {
                    while idx < str.len() && brace_level > 0 {
                        idx += 1;
                        let old_idx = idx;

                        while idx < str.len() && LexClass::of(str[idx]) == LexClass::Alpha {
                            idx += 1;
                        }

                        if idx < str.len() && idx == old_idx {
                            idx += 1;
                        } else {
                            let res = hash.lookup_str(pool, &str[old_idx..idx], StrIlk::ControlSeq);
                            if res.exists {
                                let HashExtra::ControlSeq(seq) = hash.node(res.loc).extra else {
                                    panic!("ControlSeq lookup didn't have ControlSeq extra");
                                };
                                match seq {
                                    ControlSeq::LowerSS => string_width += 500,
                                    ControlSeq::LowerAE => string_width += 722,
                                    ControlSeq::LowerOE => string_width += 778,
                                    ControlSeq::UpperAE => string_width += 903,
                                    ControlSeq::UpperOE => string_width += 1014,
                                    _ => string_width += CHAR_WIDTH[str[old_idx] as usize],
                                }
                            }
                        }

                        while idx < str.len() && LexClass::of(str[idx]) == LexClass::Whitespace {
                            idx += 1;
                        }

                        while idx < str.len() && brace_level > 0 && str[idx] != b'\\' {
                            match str[idx] {
                                b'{' => brace_level += 1,
                                b'}' => brace_level -= 1,
                                c => string_width += CHAR_WIDTH[c as usize],
                            }
                            idx += 1;
                        }
                    }

                    idx -= 1;
                } else {
                    string_width += CHAR_WIDTH[b'{' as usize];
                }
            }
            b'}' => {
                decr_brace_level(ctx, pool, cites, s1, &mut brace_level)?;
                string_width += CHAR_WIDTH[b'}' as usize];
            }
            _ => string_width += CHAR_WIDTH[str[idx] as usize],
        }

        idx += 1;
    }

    check_brace_level(ctx, pool, cites, s1, brace_level)?;
    ctx.push_stack(ExecVal::Integer(string_width));

    Ok(())
}

fn interp_write(
    ctx: &mut ExecCtx<'_, '_, '_>,
    pool: &mut StringPool,
    hash: &HashData,
    buffers: &mut GlobalBuffer,
    cites: &CiteInfo,
) -> Result<(), BibtexError> {
    let pop1 = ctx.pop_stack(pool, cites)?;
    match pop1 {
        ExecVal::String(s1) => {
            add_out_pool(ctx, buffers, pool, s1);
        }
        _ => {
            print_wrong_stk_lit(ctx, pool, hash, cites, pop1, StkType::String)?;
        }
    }
    Ok(())
}

pub(crate) fn execute_fn(
    ctx: &mut ExecCtx<'_, '_, '_>,
    globals: &mut GlobalItems<'_>,
    ex_fn_loc: HashPointer,
) -> Result<(), BibtexError> {
    match &globals.hash.node(ex_fn_loc).extra {
        HashExtra::Text => {
            ctx.push_stack(ExecVal::String(globals.hash.text(ex_fn_loc)));
            Ok(())
        }
        HashExtra::Integer(i) => {
            ctx.push_stack(ExecVal::Integer(*i));
            Ok(())
        }
        HashExtra::BstFn(BstFn::Builtin(builtin)) => match builtin {
            BstBuiltin::Eq => interp_eq(ctx, globals.pool, globals.hash, globals.cites),
            BstBuiltin::Gt => interp_gt(ctx, globals.pool, globals.hash, globals.cites),
            BstBuiltin::Lt => interp_lt(ctx, globals.pool, globals.hash, globals.cites),
            BstBuiltin::Plus => interp_plus(ctx, globals.pool, globals.hash, globals.cites),
            BstBuiltin::Minus => interp_minus(ctx, globals.pool, globals.hash, globals.cites),
            BstBuiltin::Concat => interp_concat(ctx, globals.pool, globals.hash, globals.cites),
            BstBuiltin::Set => interp_gets(
                ctx,
                globals.pool,
                globals.hash,
                globals.entries,
                globals.globals,
                globals.cites,
            ),
            BstBuiltin::AddPeriod => {
                interp_add_period(ctx, globals.pool, globals.hash, globals.cites)
            }
            BstBuiltin::CallType => {
                let default = globals.cites.get_type(globals.cites.ptr());
                if !ctx.mess_with_entries {
                    bst_cant_mess_with_entries_print(ctx, globals.pool, globals.cites)?;
                    Ok(())
                } else if default == HashData::undefined() {
                    execute_fn(ctx, globals, ctx.default)
                } else if default != 0 {
                    execute_fn(ctx, globals, default)
                } else {
                    Ok(())
                }
            }
            BstBuiltin::ChangeCase => {
                interp_change_case(ctx, globals.pool, globals.cites, globals.hash)
            }
            BstBuiltin::ChrToInt => {
                interp_chr_to_int(ctx, globals.pool, globals.hash, globals.cites)
            }
            BstBuiltin::Cite => interp_cite(ctx, globals.pool, globals.cites),
            BstBuiltin::Duplicate => interp_dup(ctx, globals.pool, globals.cites),
            BstBuiltin::Empty => interp_empty(ctx, globals.pool, globals.hash, globals.cites),
            BstBuiltin::FormatName => interp_format_name(
                ctx,
                globals.pool,
                globals.buffers,
                globals.cites,
                globals.hash,
            ),
            BstBuiltin::If => {
                let pop1 = ctx.pop_stack(globals.pool, globals.cites)?;
                let pop2 = ctx.pop_stack(globals.pool, globals.cites)?;
                let pop3 = ctx.pop_stack(globals.pool, globals.cites)?;

                match (pop1, pop2, pop3) {
                    (ExecVal::Function(f1), ExecVal::Function(f2), ExecVal::Integer(i3)) => {
                        if i3 > 0 {
                            execute_fn(ctx, globals, f2)
                        } else {
                            execute_fn(ctx, globals, f1)
                        }
                    }
                    (ExecVal::Function(_), ExecVal::Function(_), _) => print_wrong_stk_lit(
                        ctx,
                        globals.pool,
                        globals.hash,
                        globals.cites,
                        pop3,
                        StkType::Integer,
                    ),
                    (ExecVal::Function(_), _, _) => print_wrong_stk_lit(
                        ctx,
                        globals.pool,
                        globals.hash,
                        globals.cites,
                        pop2,
                        StkType::Function,
                    ),
                    (_, _, _) => print_wrong_stk_lit(
                        ctx,
                        globals.pool,
                        globals.hash,
                        globals.cites,
                        pop1,
                        StkType::Function,
                    ),
                }
            }
            BstBuiltin::IntToChr => {
                interp_int_to_chr(ctx, globals.pool, globals.hash, globals.cites)
            }
            BstBuiltin::IntToStr => {
                interp_int_to_str(ctx, globals.pool, globals.hash, globals.cites)
            }
            BstBuiltin::Missing => interp_missing(ctx, globals.pool, globals.hash, globals.cites),
            BstBuiltin::Newline => {
                output_bbl_line(ctx, globals.buffers);
                Ok(())
            }
            BstBuiltin::NumNames => interp_num_names(
                ctx,
                globals.pool,
                globals.buffers,
                globals.hash,
                globals.cites,
            ),
            BstBuiltin::Pop => ctx.pop_stack(globals.pool, globals.cites).map(|_| ()),
            BstBuiltin::Preamble => interp_preamble(ctx, globals.pool, globals.bibs),
            BstBuiltin::Purify => interp_purify(ctx, globals.pool, globals.hash, globals.cites),
            BstBuiltin::Quote => interp_quote(ctx, globals.pool),
            BstBuiltin::Skip => Ok(()),
            BstBuiltin::Stack => pop_whole_stack(ctx, globals.pool, globals.hash, globals.cites),
            BstBuiltin::Substring => interp_substr(ctx, globals.pool, globals.hash, globals.cites),
            BstBuiltin::Swap => interp_swap(ctx, globals.pool, globals.cites),
            BstBuiltin::TextLength => {
                interp_text_len(ctx, globals.pool, globals.hash, globals.cites)
            }
            BstBuiltin::TextPrefix => {
                interp_text_prefix(ctx, globals.pool, globals.hash, globals.cites)
            }
            BstBuiltin::Top => pop_top_and_print(ctx, globals.pool, globals.hash, globals.cites),
            BstBuiltin::Type => interp_ty(ctx, globals.pool, globals.hash, globals.cites),
            BstBuiltin::Warning => interp_warning(ctx, globals.pool, globals.hash, globals.cites),
            BstBuiltin::While => {
                let pop1 = ctx.pop_stack(globals.pool, globals.cites)?;
                let pop2 = ctx.pop_stack(globals.pool, globals.cites)?;

                match (pop1, pop2) {
                    (ExecVal::Function(f1), ExecVal::Function(f2)) => {
                        loop {
                            execute_fn(ctx, globals, f2)?;
                            let res = ctx.pop_stack(globals.pool, globals.cites)?;
                            if let ExecVal::Integer(i1) = res {
                                if i1 > 0 {
                                    execute_fn(ctx, globals, f1)?;
                                } else {
                                    break;
                                }
                            } else {
                                print_wrong_stk_lit(
                                    ctx,
                                    globals.pool,
                                    globals.hash,
                                    globals.cites,
                                    res,
                                    StkType::Integer,
                                )?;
                                break;
                            }
                        }
                        Ok(())
                    }
                    (ExecVal::Function(_), _) => print_wrong_stk_lit(
                        ctx,
                        globals.pool,
                        globals.hash,
                        globals.cites,
                        pop2,
                        StkType::Function,
                    ),
                    (_, _) => print_wrong_stk_lit(
                        ctx,
                        globals.pool,
                        globals.hash,
                        globals.cites,
                        pop1,
                        StkType::Function,
                    ),
                }
            }
            BstBuiltin::Width => interp_width(ctx, globals.pool, globals.hash, globals.cites),
            BstBuiltin::Write => interp_write(
                ctx,
                globals.pool,
                globals.hash,
                globals.buffers,
                globals.cites,
            ),
        },
        HashExtra::BstFn(BstFn::Wizard(mut wiz_ptr)) => {
            let mut cur_fn = globals.other.wiz_function(wiz_ptr);
            while cur_fn != HashData::end_of_def() {
                if cur_fn != QUOTE_NEXT_FN {
                    execute_fn(ctx, globals, cur_fn)?;
                } else {
                    wiz_ptr += 1;
                    cur_fn = globals.other.wiz_function(wiz_ptr);
                    ctx.push_stack(ExecVal::Function(cur_fn))
                }
                wiz_ptr += 1;
                cur_fn = globals.other.wiz_function(wiz_ptr);
            }
            Ok(())
        }
        HashExtra::BstFn(BstFn::Field(field)) => {
            if !ctx.mess_with_entries {
                bst_cant_mess_with_entries_print(ctx, globals.pool, globals.cites)
            } else {
                let field_ptr = globals.cites.ptr() * globals.other.num_fields() + *field;
                if field_ptr >= globals.other.max_fields() {
                    ctx.write_logs("field_info index is out of range");
                    print_confusion(ctx);
                    return Err(BibtexError::Fatal);
                }

                let field = globals.other.field(field_ptr);
                if field.is_invalid() {
                    ctx.push_stack(ExecVal::Missing(globals.hash.text(ex_fn_loc)));
                } else {
                    ctx.push_stack(ExecVal::String(field));
                }
                Ok(())
            }
        }
        HashExtra::BstFn(BstFn::IntEntry(entry)) => {
            if !ctx.mess_with_entries {
                bst_cant_mess_with_entries_print(ctx, globals.pool, globals.cites)
            } else {
                ctx.push_stack(ExecVal::Integer(
                    globals
                        .entries
                        .ints(globals.cites.ptr() * globals.entries.num_ent_ints() + *entry),
                ));
                Ok(())
            }
        }
        HashExtra::BstFn(BstFn::StrEntry(entry)) => {
            if !ctx.mess_with_entries {
                bst_cant_mess_with_entries_print(ctx, globals.pool, globals.cites)
            } else {
                let str_ent_ptr = globals.cites.ptr() * globals.entries.num_ent_strs() + *entry;
                let str = globals.entries.strs(str_ent_ptr);
                let val = ExecVal::String(globals.pool.add_string(str));
                ctx.push_stack(val);
                Ok(())
            }
        }
        HashExtra::BstFn(BstFn::IntGlbl(value)) => {
            ctx.push_stack(ExecVal::Integer(*value));
            Ok(())
        }
        HashExtra::BstFn(BstFn::StrGlbl(glb_ptr)) => {
            let str_ptr = globals.globals.str_ptr(*glb_ptr);
            if !str_ptr.is_invalid() {
                ctx.push_stack(ExecVal::String(str_ptr));
            } else {
                let str = globals.globals.str(*glb_ptr);
                let val = ExecVal::String(globals.pool.add_string(str));
                ctx.push_stack(val);
            }
            Ok(())
        }
        _ => panic!("Invalid node passed as ex_fn_loc"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_empty_sl_range() {
        let slice = b"0123456789";

        let r1 = SLRange { start: 0, len: 0 };
        assert_eq!(&slice[r1], &[]);
        let r2 = SLRange { start: 5, len: 0 };
        assert_eq!(&slice[r2], &[]);
        let r3 = SLRange { start: -5, len: 0 };
        assert_eq!(&slice[r3], &[]);
    }

    #[test]
    fn test_sl_range() {
        let slice = b"0123456789";

        let r1 = SLRange { start: 1, len: 5 };
        assert_eq!(&slice[r1], b"01234");
        let r2 = SLRange { start: 3, len: 2 };
        assert_eq!(&slice[r2], b"23");
        let r3 = SLRange { start: -1, len: 2 };
        assert_eq!(&slice[r3], b"89");
    }

    #[test]
    fn test_sl_range_long() {
        let slice = b"0123456789";

        let r1 = SLRange { start: 1, len: 100 };
        assert_eq!(&slice[r1], b"0123456789");

        let r1 = SLRange {
            start: -1,
            len: 100,
        };
        assert_eq!(&slice[r1], b"0123456789");
    }
}
