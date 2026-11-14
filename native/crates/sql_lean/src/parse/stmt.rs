//! The program and its statements: queries (`_dml_read`), writes (`_dml_write`), and the
//! definitions the parser reads (`create table`, `create index`, `create view`, `drop`, `alter
//! table`'s common forms, `set`, `reset`, `comment on`). Any other statement is refused.

use super::expr::AFTER_TERM;
use super::*;

/// The keywords a relation's alias may not be: what may follow a relation.
const AFTER_RELATION: &[K] = AFTER_TERM;

/// The type keywords `_param_name` takes as an identifier (`@identifier{keyword_text}`, …).
const PARAM_NAMES: &[K] = &[
    K::KeywordImage,
    K::KeywordSmallserial,
    K::KeywordSerial,
    K::KeywordBigserial,
    K::KeywordMoney,
    K::KeywordSmallmoney,
    K::KeywordString,
    K::KeywordText,
    K::KeywordUuid,
    K::KeywordJsonb,
    K::KeywordXml,
    K::KeywordBytea,
    K::KeywordInet,
    K::KeywordDate,
    K::KeywordDatetime,
    K::KeywordDatetime2,
    K::KeywordSmalldatetime,
    K::KeywordTimestamptz,
    K::KeywordGeometry,
    K::KeywordGeography,
    K::KeywordBox2d,
    K::KeywordBox3d,
    K::KeywordOid,
    K::KeywordName,
    K::KeywordRegclass,
    K::KeywordRegnamespace,
    K::KeywordRegproc,
    K::KeywordRegtype,
];

impl P<'_> {
    /// `program`: statements, each but the last ended by `;`.
    pub(super) fn program(&mut self) -> R<u32> {
        let m = self.a.mark();
        loop {
            let t = self.peek()?;
            if t.t == T::Eof {
                break;
            }
            if self.send()? {
                continue;
            }
            self.statement()?;
            if self.punct(b";", K::TSemi)?.is_none() && !self.send()? {
                if self.peek()?.t != T::Eof {
                    return self.refuse();
                }
                break;
            }
        }
        let id = self.a.node(K::Program, m);
        Ok(id)
    }

    /// A psql command that sends the query buffer (`\\gset`), where one is next: `program`'s
    /// `meta_command`, in a statement's `;` place.
    fn send(&mut self) -> R<bool> {
        let t = self.peek()?;
        if t.t != T::Punct || self.byte(t.s) != b'\\' {
            return Ok(false);
        }
        let e = self.send_end(t.s);
        self.take(Tok { e, ..t });
        self.a.leaf(K::MetaCommand, 0, t.s, e);
        Ok(true)
    }

    /// `statement`.
    fn statement(&mut self) -> R<()> {
        self.depth = 0;
        let m = self.a.mark();
        let t = self.peek()?;
        if t.t != T::Word {
            return self.refuse();
        }
        let ws = self.words()?;
        let Some(&(_, k)) = ws.first() else { return self.refuse() };
        if ws.len() > 1 {
            return self.refuse();
        }
        match k {
            K::KeywordSelect | K::KeywordValues => self.dml_read()?,
            K::KeywordWith => {
                self.cte_list()?;
                if self.is(K::KeywordInsert)? || self.is(K::KeywordUpdate)? || self.is(K::KeywordDelete)? {
                    self.dml_write()?;
                } else {
                    self.query()?;
                }
            }
            K::KeywordInsert | K::KeywordUpdate | K::KeywordDelete => self.dml_write()?,
            K::KeywordCreate => self.create()?,
            K::KeywordDrop => self.drop()?,
            K::KeywordAlter => self.alter()?,
            K::KeywordSet => self.set_statement()?,
            K::KeywordReset => self.reset_statement()?,
            K::KeywordComment => self.comment_statement()?,
            _ => return self.refuse(),
        }
        self.a.node(K::Statement, m);
        Ok(())
    }

    /// `_dml_read`: a query, its common table expressions first.
    pub(super) fn dml_read(&mut self) -> R<()> {
        if self.is(K::KeywordWith)? {
            self.cte_list()?;
        }
        self.query()
    }

    /// `_cte`: `with [recursive] a as (…), …`.
    fn cte_list(&mut self) -> R<()> {
        self.expect(K::KeywordWith)?;
        if self.is(K::KeywordRecursive)? {
            self.expect(K::KeywordRecursive)?;
        }
        loop {
            let m = self.a.mark();
            self.expect_ident(NONE, 0)?;
            if self.at_punct(b"(")? {
                self.expect_punct(b"(", K::TLParen)?;
                loop {
                    self.expect_ident(NONE, f::ARGUMENT)?;
                    if self.punct(b",", K::TComma)?.is_none() {
                        break;
                    }
                }
                self.expect_punct(b")", K::TRParen)?;
            }
            self.expect(K::KeywordAs)?;
            if self.is(K::KeywordNot)? || self.is(K::KeywordMaterialized)? {
                return self.refuse();
            }
            self.expect_punct(b"(", K::TLParen)?;
            let s = self.a.mark();
            if self.is(K::KeywordInsert)? || self.is(K::KeywordUpdate)? || self.is(K::KeywordDelete)? {
                self.dml_write()?;
            } else {
                self.dml_read()?;
            }
            self.a.node(K::Statement, s);
            self.expect_punct(b")", K::TRParen)?;
            if self.is(K::KeywordSearch)? || self.is(K::KeywordCycle)? {
                return self.refuse();
            }
            self.a.node(K::Cte, m);
            if self.punct(b",", K::TComma)?.is_none() {
                break;
            }
        }
        Ok(())
    }

    /// A query: a select statement, or a set operation of them.
    fn query(&mut self) -> R<()> {
        let m = self.a.mark();
        self.select_statement()?;
        if self.is(K::KeywordUnion)? || self.is(K::KeywordExcept)? || self.is(K::KeywordIntersect)? {
            loop {
                if self.kw_field(K::KeywordUnion, f::OPERATION)?.is_some() {
                    if self.kw_field(K::KeywordAll, f::OPERATION)?.is_none() {
                        self.kw_field(K::KeywordDistinct, f::OPERATION)?;
                    }
                } else if self.kw_field(K::KeywordExcept, f::OPERATION)?.is_some() || self.kw_field(K::KeywordIntersect, f::OPERATION)?.is_some() {
                    self.kw_field(K::KeywordDistinct, f::OPERATION)?;
                } else {
                    break;
                }
                self.select_statement()?;
            }
            self.tail()?;
            self.a.node(K::SetOperation, m);
        }
        Ok(())
    }

    /// `_select_statement`: `select …` (with its `from`), `values …`, or one in parentheses.
    fn select_statement(&mut self) -> R<()> {
        self.deeper()?;
        if self.punct(b"(", K::TLParen)?.is_some() {
            if !self.is(K::KeywordSelect)? && !self.is(K::KeywordValues)? {
                return self.refuse();
            }
            self.select_statement()?;
            self.expect_punct(b")", K::TRParen)?;
        } else if self.is(K::KeywordValues)? {
            self.values()?;
            self.tail()?;
        } else {
            self.select()?;
            if self.is(K::KeywordInto)? {
                return self.refuse();
            }
            if self.is(K::KeywordFrom)? {
                self.from()?;
            } else {
                if self.is(K::KeywordWhere)? {
                    self.where_()?;
                }
                self.group_or_having()?;
                if self.is(K::KeywordWindow)? {
                    return self.refuse();
                }
                self.tail()?;
            }
        }
        self.depth -= 1;
        Ok(())
    }

    /// `values (…), (…)`.
    fn values(&mut self) -> R<()> {
        let m = self.a.mark();
        self.expect(K::KeywordValues)?;
        loop {
            if !self.at_punct(b"(")? {
                return self.refuse();
            }
            let id = self.paren(false)?;
            match self.a.nodes[id as usize].kind {
                K::List => {}
                K::ParenthesizedExpression => self.a.nodes[id as usize].kind = K::List,
                _ => return self.refuse(),
            }
            if self.punct(b",", K::TComma)?.is_none() {
                break;
            }
        }
        self.a.node(K::Values, m);
        Ok(())
    }

    /// `select [all | distinct [on (…)]] terms`.
    fn select(&mut self) -> R<()> {
        let m = self.a.mark();
        self.expect(K::KeywordSelect)?;
        if self.kw(K::KeywordAll)?.is_none() && self.kw(K::KeywordDistinct)?.is_some() && self.kw(K::KeywordOn)?.is_some() {
            self.expect_punct(b"(", K::TLParen)?;
            loop {
                self.expr(0, 0)?;
                if self.punct(b",", K::TComma)?.is_none() {
                    break;
                }
            }
            self.expect_punct(b")", K::TRParen)?;
        }
        if !self.ends_select()? {
            self.select_expression()?;
        }
        self.a.node(K::Select, m);
        Ok(())
    }

    /// Whether the select list is empty: the next token ends the select.
    fn ends_select(&mut self) -> R<bool> {
        let t = self.peek()?;
        Ok(match t.t {
            T::Eof => true,
            T::Punct => matches!(self.byte(t.s), b';' | b')'),
            T::Word => {
                let ws = self.words()?;
                ws.iter().any(|&(_, k)| matches!(k, K::KeywordFrom | K::KeywordWhere | K::KeywordUnion | K::KeywordExcept | K::KeywordIntersect | K::KeywordOrder | K::KeywordLimit | K::KeywordGroup | K::KeywordInto))
            }
            _ => false,
        })
    }

    /// `select_expression`: terms, `,` between them.
    pub(super) fn select_expression(&mut self) -> R<u32> {
        let m = self.a.mark();
        loop {
            self.term()?;
            if self.punct(b",", K::TComma)?.is_none() {
                break;
            }
        }
        Ok(self.a.node(K::SelectExpression, m))
    }

    /// `from`: relations and joins, then the clauses after them.
    fn from(&mut self) -> R<()> {
        let m = self.a.mark();
        self.expect(K::KeywordFrom)?;
        self.relation()?;
        loop {
            if self.punct(b",", K::TComma)?.is_some() {
                self.relation()?;
            } else if !self.join()? {
                break;
            }
        }
        if self.is(K::KeywordWhere)? {
            self.where_()?;
        }
        self.group_or_having()?;
        if self.is(K::KeywordWindow)? {
            return self.refuse();
        }
        self.tail()?;
        self.a.node(K::From, m);
        Ok(())
    }

    /// `where predicate`.
    fn where_(&mut self) -> R<()> {
        let m = self.a.mark();
        self.expect(K::KeywordWhere)?;
        self.expr(0, f::PREDICATE)?;
        self.a.node(K::Where, m);
        Ok(())
    }

    /// `group by …` (with its `having`), or a `having` alone.
    fn group_or_having(&mut self) -> R<()> {
        if self.is(K::KeywordGroup)? {
            let m = self.a.mark();
            self.expect(K::KeywordGroup)?;
            self.expect(K::KeywordBy)?;
            if self.is(K::KeywordAll)? || self.is(K::KeywordDistinct)? || self.is(K::KeywordCube)? || self.is(K::KeywordRollup)? || self.is(K::KeywordGrouping)? {
                return self.refuse();
            }
            loop {
                self.expr(0, 0)?;
                if self.punct(b",", K::TComma)?.is_none() {
                    break;
                }
            }
            if self.kw(K::KeywordHaving)?.is_some() {
                self.expr(0, 0)?;
            }
            self.a.node(K::GroupBy, m);
        } else if self.kw(K::KeywordHaving)?.is_some() {
            self.expr(0, 0)?;
        }
        Ok(())
    }

    /// The clauses that end a query: `order by`, `limit`, `offset`; a `fetch` or `for` is refused.
    fn tail(&mut self) -> R<()> {
        if self.is(K::KeywordOrder)? {
            let m = self.a.mark();
            self.expect(K::KeywordOrder)?;
            self.expect(K::KeywordBy)?;
            loop {
                let t = self.a.mark();
                self.expr(0, 0)?;
                if self.kw(K::KeywordAsc)?.is_some() || self.kw(K::KeywordDesc)?.is_some() {
                    let d = self.a.mark() - 1;
                    self.a.node(K::Direction, d);
                } else if self.is(K::KeywordUsing)? {
                    return self.refuse();
                }
                if self.kw(K::KeywordNulls)?.is_some() && self.kw(K::KeywordFirst)?.is_none() {
                    self.expect(K::KeywordLast)?;
                }
                self.a.node(K::OrderTarget, t);
                if self.punct(b",", K::TComma)?.is_none() {
                    break;
                }
            }
            self.a.node(K::OrderBy, m);
        }
        if self.is(K::KeywordLimit)? {
            let m = self.a.mark();
            self.expect(K::KeywordLimit)?;
            if self.kw(K::KeywordAll)?.is_none() {
                self.expr(0, 0)?;
            }
            if self.is(K::KeywordOffset)? {
                self.offset()?;
            }
            self.a.node(K::Limit, m);
        } else if self.is(K::KeywordOffset)? {
            self.offset()?;
            if self.is(K::KeywordLimit)? || self.is(K::KeywordFetch)? {
                return self.refuse();
            }
        }
        if self.is(K::KeywordFetch)? {
            // `fetch_clause`: `fetch first|next [n] row|rows only|with ties`.
            let m = self.a.mark();
            self.expect(K::KeywordFetch)?;
            if self.kw(K::KeywordFirst)?.is_none() {
                self.expect(K::KeywordNext)?;
            }
            if !self.is(K::KeywordRow)? && !self.is(K::KeywordRows)? {
                self.expr(0, 0)?;
            }
            if self.kw(K::KeywordRow)?.is_none() {
                self.expect(K::KeywordRows)?;
            }
            if self.kw(K::KeywordOnly)?.is_none() {
                self.expect(K::KeywordWith)?;
                self.expect(K::KeywordTies)?;
            }
            self.a.node(K::FetchClause, m);
        }
        if self.is(K::KeywordFor)? {
            return self.refuse();
        }
        Ok(())
    }

    fn offset(&mut self) -> R<()> {
        let m = self.a.mark();
        self.expect(K::KeywordOffset)?;
        self.expr(0, 0)?;
        if self.kw(K::KeywordRow)?.is_none() {
            self.kw(K::KeywordRows)?;
        }
        self.a.node(K::Offset, m);
        Ok(())
    }

    /// `relation`: a table (with its alias), a subquery, or a function.
    fn relation(&mut self) -> R<()> {
        let m = self.a.mark();
        if self.at_punct(b"(")? {
            let save = (self.at, self.last_end, self.look);
            let t = self.peek()?;
            self.take(Tok { e: t.s + 1, ..t });
            let values = self.is(K::KeywordValues)?;
            (self.at, self.last_end, self.look) = save;
            if values {
                // `(values …)`: the relation's own parentheses.
                self.expect_punct(b"(", K::TLParen)?;
                self.values()?;
                self.expect_punct(b")", K::TRParen)?;
            } else {
                let id = self.paren(false)?;
                if self.a.nodes[id as usize].kind != K::Subquery {
                    return self.refuse();
                }
            }
            self.relation_alias()?;
        } else {
            if self.is(K::KeywordOnly)? || self.is(K::KeywordLateral)? || self.is(K::KeywordRows)? {
                return self.refuse();
            }
            let Some(r) = self.object_reference(Ctx::Sure(&[&[K::KeywordOnly, K::KeywordLateral, K::KeywordRows]]), 0)? else { return self.refuse() };
            if self.at_punct(b"(")? {
                // A function in `from` (`_func_table`): an invocation of the reference.
                let _ = r;
                return self.refuse();
            }
            if self.at_punct(b"*")? || self.is(K::KeywordTablesample)? {
                return self.refuse();
            }
            self.relation_alias()?;
        }
        self.a.node(K::Relation, m);
        Ok(())
    }

    /// `_relation_alias`: an alias and its column list.
    fn relation_alias(&mut self) -> R<()> {
        let before = self.a.mark();
        if self.kw(K::KeywordAs)?.is_some() {
            let t = self.peek()?;
            if t.t == T::Str && self.byte(t.s) == b'\'' {
                return self.refuse();
            }
            self.expect_ident(NONE, f::ALIAS)?;
        } else {
            self.ident(Ctx::Unsure(&[AFTER_RELATION]), f::ALIAS)?;
        }
        if self.a.mark() > before && self.at_punct(b"(")? {
            self.column_list()?;
        }
        Ok(())
    }

    /// `_column_list` as a `list`: `(a, b)`, each a `column`.
    fn column_list(&mut self) -> R<u32> {
        let m = self.a.mark();
        self.expect_punct(b"(", K::TLParen)?;
        loop {
            let c = self.a.mark();
            let t = self.peek()?;
            if t.t == T::Str {
                return self.refuse();
            }
            self.expect_ident(NONE, 0)?;
            self.a.node(K::Column, c);
            if self.punct(b",", K::TComma)?.is_none() {
                break;
            }
        }
        self.expect_punct(b")", K::TRParen)?;
        Ok(self.a.node(K::List, m))
    }

    /// A join, where one is next: `[natural] [left|right|full [outer]|inner] join relation [on …|using (…)]`.
    fn join(&mut self) -> R<bool> {
        let ws = self.words()?;
        let joins = [K::KeywordNatural, K::KeywordLeft, K::KeywordRight, K::KeywordFull, K::KeywordInner, K::KeywordJoin, K::KeywordCross];
        if !ws.iter().any(|&(_, k)| joins.contains(&k)) {
            return Ok(false);
        }
        let m = self.a.mark();
        if self.is(K::KeywordCross)? {
            self.expect(K::KeywordCross)?;
            self.expect(K::KeywordJoin)?;
            if self.is(K::KeywordLateral)? {
                return self.refuse();
            }
            self.relation()?;
            self.a.node(K::CrossJoin, m);
            return Ok(true);
        }
        self.kw(K::KeywordNatural)?;
        if self.kw(K::KeywordLeft)?.is_some() || self.kw(K::KeywordRight)?.is_some() || self.kw(K::KeywordFull)?.is_some() {
            self.kw(K::KeywordOuter)?;
        } else {
            self.kw(K::KeywordInner)?;
        }
        self.expect(K::KeywordJoin)?;
        if self.is(K::KeywordLateral)? {
            return self.refuse();
        }
        self.relation()?;
        if self.kw(K::KeywordOn)?.is_some() {
            self.expr(0, f::PREDICATE)?;
        } else if self.kw(K::KeywordUsing)?.is_some() {
            self.column_list()?;
            if self.is(K::KeywordAs)? {
                return self.refuse();
            }
        } else {
            // A join with no condition: a join after it would be the grammar's nested `join?`,
            // which tree-sitter's GLR decides.
            let ws = self.words()?;
            if ws.iter().any(|&(_, k)| joins.contains(&k)) {
                return self.refuse();
            }
        }
        self.a.node(K::Join, m);
        Ok(true)
    }

    // ---- Writes.

    /// `_dml_write`: `insert`, `update`, `delete`.
    fn dml_write(&mut self) -> R<()> {
        if self.is(K::KeywordInsert)? {
            self.insert()?;
        } else if self.is(K::KeywordUpdate)? {
            self.update()?;
        } else {
            self.delete()?;
        }
        if self.is(K::KeywordReturning)? {
            let m = self.a.mark();
            self.expect(K::KeywordReturning)?;
            self.select_expression()?;
            self.a.node(K::Returning, m);
        }
        Ok(())
    }

    /// `insert into t [(columns)] values … | query`.
    fn insert(&mut self) -> R<()> {
        let m = self.a.mark();
        self.expect(K::KeywordInsert)?;
        self.expect(K::KeywordInto)?;
        self.object_reference(NONE, 0)?.map_or_else(|| self.refuse(), |_| Ok(()))?;
        if self.is(K::KeywordAs)? || self.is(K::KeywordPartition)? || self.is(K::KeywordSet)? || self.is(K::KeywordDefault)? {
            return self.refuse();
        }
        if self.at_punct(b"(")? {
            // A column list, or a query in parentheses.
            let save = (self.at, self.last_end, self.look);
            let t = self.peek()?;
            self.take(Tok { e: t.s + 1, ..t });
            let query = self.is(K::KeywordSelect)? || self.is(K::KeywordWith)? || self.is(K::KeywordValues)?;
            (self.at, self.last_end, self.look) = save;
            if query {
                return self.refuse();
            }
            self.column_list()?;
        }
        if self.is(K::KeywordOverriding)? {
            return self.refuse();
        }
        if self.is(K::KeywordValues)? {
            self.expect(K::KeywordValues)?;
            loop {
                if !self.at_punct(b"(")? {
                    return self.refuse();
                }
                let id = self.paren(false)?;
                match self.a.nodes[id as usize].kind {
                    K::List => {}
                    K::ParenthesizedExpression => self.a.nodes[id as usize].kind = K::List,
                    _ => return self.refuse(),
                }
                if self.punct(b",", K::TComma)?.is_none() {
                    break;
                }
            }
        } else if self.is(K::KeywordSelect)? || self.is(K::KeywordWith)? {
            self.dml_read()?;
        } else {
            return self.refuse();
        }
        if self.is(K::KeywordOn)? {
            return self.refuse();
        }
        self.a.node(K::Insert, m);
        Ok(())
    }

    /// `update t set a = …, … [from …] [where …]`.
    fn update(&mut self) -> R<()> {
        let m = self.a.mark();
        self.expect(K::KeywordUpdate)?;
        if self.is(K::KeywordOnly)? {
            return self.refuse();
        }
        self.relation()?;
        if self.at_punct(b",")? {
            return self.refuse();
        }
        self.set_values()?;
        if self.is(K::KeywordFrom)? {
            self.from()?;
        } else if self.is(K::KeywordWhere)? {
            self.where_()?;
        }
        self.a.node(K::Update, m);
        Ok(())
    }

    /// `_set_values`: `set a = x, …`.
    fn set_values(&mut self) -> R<()> {
        self.expect(K::KeywordSet)?;
        loop {
            let m = self.a.mark();
            if self.at_punct(b"(")? {
                return self.refuse();
            }
            // `_set_target`: a field.
            let n = self.a.mark();
            let first = self.expect_ident(NONE, f::NAME)?;
            let mut parts = vec![first];
            while self.at_punct(b".")? {
                let t = self.peek()?;
                self.take(Tok { e: t.s + 1, ..t });
                parts.push(self.expect_ident(NONE, f::NAME)?);
            }
            self.a.stack.truncate(n);
            match parts.len() {
                1 => self.a.stack.push(parts[0]),
                2 | 3 => {
                    let (head, name) = parts.split_at(parts.len() - 1);
                    self.push_reference(head, 0);
                    let d = self.dot_before(name[0]);
                    self.a.leaf(K::TDot, 0, d, d + 1);
                    self.a.stack.push(name[0]);
                }
                _ => return self.refuse(),
            }
            let id = self.a.node(K::Field, n);
            self.a.set_field(id, f::LEFT);
            if self.at_punct(b"[")? {
                return self.refuse();
            }
            self.expect_punct(b"=", K::TEq)?;
            self.expr(0, f::RIGHT)?;
            self.a.node(K::Assignment, m);
            if self.punct(b",", K::TComma)?.is_none() {
                break;
            }
        }
        Ok(())
    }

    /// `delete from t [where …]`: `delete`, then `_delete_from` as a `from`.
    fn delete(&mut self) -> R<()> {
        let m = self.a.mark();
        self.expect(K::KeywordDelete)?;
        self.a.node(K::Delete, m);
        let m = self.a.mark();
        self.expect(K::KeywordFrom)?;
        if self.is(K::KeywordOnly)? {
            return self.refuse();
        }
        self.object_reference(NONE, 0)?.map_or_else(|| self.refuse(), |_| Ok(()))?;
        if self.at_punct(b"*")? {
            return self.refuse();
        }
        self.alias_unsure()?;
        if self.is(K::KeywordUsing)? {
            return self.refuse();
        }
        if self.is(K::KeywordWhere)? {
            self.where_()?;
        }
        if self.is(K::KeywordOrder)? || self.is(K::KeywordLimit)? {
            return self.refuse();
        }
        self.a.node(K::From, m);
        Ok(())
    }

    /// An `_alias` where what may follow is a clause.
    fn alias_unsure(&mut self) -> R<()> {
        if self.kw(K::KeywordAs)?.is_some() {
            self.expect_ident(NONE, f::ALIAS)?;
        } else {
            self.ident(Ctx::Unsure(&[AFTER_TERM]), f::ALIAS)?;
        }
        Ok(())
    }

    // ---- Definitions.

    fn create(&mut self) -> R<()> {
        let save = (self.at, self.last_end, self.look);
        let t = self.peek()?;
        self.take(t);
        let next = self.words()?;
        let mut kinds: Vec<K> = next.iter().map(|&(_, k)| k).collect();
        if kinds.contains(&K::KeywordOr) || kinds.contains(&K::KeywordTemp) || kinds.contains(&K::KeywordTemporary) || kinds.contains(&K::KeywordUnlogged) {
            // `create or replace view`, `create temporary table`: the word after decides.
            let t = self.peek()?;
            self.take(t);
            if kinds.contains(&K::KeywordOr) {
                let t = self.peek()?;
                self.take(t);
            }
            kinds = self.words()?.iter().map(|&(_, k)| k).collect();
        }
        (self.at, self.last_end, self.look) = save;
        if kinds.contains(&K::KeywordTable) {
            self.create_table()
        } else if kinds.contains(&K::KeywordIndex) || kinds.contains(&K::KeywordUnique) {
            self.create_index()
        } else if kinds.contains(&K::KeywordView) {
            self.create_view()
        } else if kinds.contains(&K::KeywordFunction) {
            self.create_function()
        } else if kinds.contains(&K::KeywordExtension) {
            self.create_extension()
        } else if kinds.contains(&K::KeywordSchema) {
            self.create_schema()
        } else {
            self.refuse()
        }
    }

    /// `create [temporary | unlogged] table [if not exists] t (columns and constraints) | as query`.
    fn create_table(&mut self) -> R<()> {
        let m = self.a.mark();
        self.expect(K::KeywordCreate)?;
        if self.kw(K::KeywordTemp)?.is_none() && self.kw(K::KeywordTemporary)?.is_none() {
            self.kw(K::KeywordUnlogged)?;
        }
        self.expect(K::KeywordTable)?;
        self.if_not_exists()?;
        if self.object_reference(Ctx::Sure(&[&[K::KeywordIf]]), 0)?.is_none() {
            return self.refuse();
        }
        if self.at_punct(b"(")? {
            self.table_elements()?;
            if self.is(K::KeywordAs)? {
                return self.refuse();
            }
        } else if self.is(K::KeywordAs)? {
            self.expect(K::KeywordAs)?;
            let q = self.a.mark();
            if !self.is(K::KeywordSelect)? && !self.is(K::KeywordWith)? && !self.is(K::KeywordValues)? {
                return self.refuse();
            }
            self.dml_read()?;
            self.a.node(K::CreateQuery, q);
            if self.is(K::KeywordWith)? {
                return self.refuse();
            }
        } else {
            return self.refuse();
        }
        let ws = self.words()?;
        if !ws.is_empty() {
            return self.refuse();
        }
        self.a.node(K::CreateTable, m);
        Ok(())
    }

    fn if_not_exists(&mut self) -> R<()> {
        if self.kw(K::KeywordIf)?.is_some() {
            self.expect(K::KeywordNot)?;
            self.expect(K::KeywordExists)?;
        }
        Ok(())
    }

    fn if_exists(&mut self) -> R<()> {
        if self.kw(K::KeywordIf)?.is_some() {
            self.expect(K::KeywordExists)?;
        }
        Ok(())
    }

    /// `_table_elements` as `column_definitions`: `(column definitions, constraints)`.
    fn table_elements(&mut self) -> R<()> {
        let m = self.a.mark();
        self.expect_punct(b"(", K::TLParen)?;
        if !self.at_punct(b")")? {
            loop {
                let ws = self.words()?;
                let starts: &[K] = &[K::KeywordConstraint, K::KeywordPrimary, K::KeywordUnique, K::KeywordCheck, K::KeywordForeign, K::KeywordExclude, K::KeywordLike];
                if ws.iter().any(|&(_, k)| starts.contains(&k)) {
                    self.table_constraint()?;
                } else {
                    self.column_definition()?;
                }
                if self.punct(b",", K::TComma)?.is_none() {
                    break;
                }
            }
        }
        self.expect_punct(b")", K::TRParen)?;
        self.a.node(K::ColumnDefinitions, m);
        Ok(())
    }

    /// `column_definition`: a name, a type, its constraints.
    fn column_definition(&mut self) -> R<()> {
        let m = self.a.mark();
        let t = self.peek()?;
        if t.t == T::Str {
            return self.refuse();
        }
        let ws = self.words()?;
        if ws.iter().any(|&(_, k)| matches!(k, K::KeywordKey | K::KeywordIndex)) {
            // `@identifier{keyword_key | keyword_index}`: an identifier.
            self.take(t);
            self.a.leaf(K::Identifier, f::NAME, t.s, t.e);
        } else {
            self.expect_ident(Ctx::Sure(&[&[K::KeywordConstraint, K::KeywordPrimary, K::KeywordUnique, K::KeywordCheck, K::KeywordForeign, K::KeywordExclude, K::KeywordLike, K::KeywordNot]]), f::NAME)?;
        }
        self.type_(f::TYPE)?;
        loop {
            if !self.column_constraint()? {
                break;
            }
        }
        self.a.node(K::ColumnDefinition, m);
        Ok(())
    }

    /// One of a column's constraints, where one is next.
    fn column_constraint(&mut self) -> R<bool> {
        let ws = self.words()?;
        let Some(&(_, k)) = ws.first() else { return Ok(false) };
        match k {
            K::KeywordNull => {
                self.expect(k)?;
            }
            K::KeywordNot => {
                self.expect(K::KeywordNot)?;
                if self.is(K::KeywordDeferrable)? || self.is(K::KeywordEnforced)? {
                    return self.refuse();
                }
                self.expect(K::KeywordNull)?;
            }
            K::KeywordDefault => {
                self.expect(K::KeywordDefault)?;
                self.expr(prec::MINUS - 1, 0)?;
            }
            K::KeywordPrimary => {
                self.expect(K::KeywordPrimary)?;
                self.expect(K::KeywordKey)?;
                if self.is(K::KeywordInclude)? || self.is(K::KeywordWith)? || self.is(K::KeywordUsing)? {
                    return self.refuse();
                }
            }
            K::KeywordUnique => {
                self.expect(K::KeywordUnique)?;
                if self.is(K::KeywordNulls)? || self.is(K::KeywordInclude)? || self.is(K::KeywordWith)? || self.is(K::KeywordUsing)? {
                    return self.refuse();
                }
            }
            K::KeywordCheck => {
                self.expect(K::KeywordCheck)?;
                self.expect_punct(b"(", K::TLParen)?;
                self.expr(0, 0)?;
                self.expect_punct(b")", K::TRParen)?;
                if self.is(K::KeywordNo)? {
                    return self.refuse();
                }
            }
            K::KeywordReferences => self.references()?,
            _ => return Ok(false),
        }
        Ok(true)
    }

    /// `_references`: `references t [(columns)] [on delete|update action]…`.
    fn references(&mut self) -> R<()> {
        self.expect(K::KeywordReferences)?;
        if self.object_reference(NONE, 0)?.is_none() {
            return self.refuse();
        }
        if self.punct(b"(", K::TLParen)?.is_some() {
            loop {
                self.expect_ident(NONE, 0)?;
                if self.punct(b",", K::TComma)?.is_none() {
                    break;
                }
            }
            self.expect_punct(b")", K::TRParen)?;
        }
        if self.is(K::KeywordMatch)? {
            return self.refuse();
        }
        while self.kw(K::KeywordOn)?.is_some() {
            if self.kw(K::KeywordDelete)?.is_none() {
                self.expect(K::KeywordUpdate)?;
            }
            if self.kw(K::KeywordNo)?.is_some() {
                self.expect(K::KeywordAction)?;
            } else if self.kw(K::KeywordRestrict)?.is_none() && self.kw(K::KeywordCascade)?.is_none() {
                self.expect(K::KeywordSet)?;
                if self.kw(K::KeywordNull)?.is_none() {
                    self.expect(K::KeywordDefault)?;
                }
                if self.at_punct(b"(")? {
                    return self.refuse();
                }
            }
        }
        Ok(())
    }

    /// A table's `constraint`: `[constraint name] primary key (…) | unique (…) | foreign key (…) references … | check (…)`.
    fn table_constraint(&mut self) -> R<()> {
        let m = self.a.mark();
        if self.kw(K::KeywordConstraint)?.is_some() {
            self.expect_ident(NONE, f::NAME)?;
        }
        let ws = self.words()?;
        let Some(&(_, k)) = ws.first() else { return self.refuse() };
        match k {
            K::KeywordPrimary => {
                self.expect(K::KeywordPrimary)?;
                self.expect(K::KeywordKey)?;
                self.ordered_columns()?;
            }
            K::KeywordUnique => {
                self.expect(K::KeywordUnique)?;
                if !self.at_punct(b"(")? {
                    return self.refuse();
                }
                self.ordered_columns()?;
            }
            K::KeywordForeign => {
                self.expect(K::KeywordForeign)?;
                self.expect(K::KeywordKey)?;
                if !self.at_punct(b"(")? {
                    return self.refuse();
                }
                self.ordered_columns()?;
                if self.is(K::KeywordReferences)? {
                    self.references()?;
                }
            }
            K::KeywordCheck => {
                self.expect(K::KeywordCheck)?;
                self.expect_punct(b"(", K::TLParen)?;
                self.expr(0, 0)?;
                self.expect_punct(b")", K::TRParen)?;
            }
            _ => return self.refuse(),
        }
        if self.is(K::KeywordInclude)? || self.is(K::KeywordWith)? || self.is(K::KeywordUsing)? || self.is(K::KeywordDeferrable)? || self.is(K::KeywordNot)? || self.is(K::KeywordInitially)? {
            return self.refuse();
        }
        self.a.node(K::Constraint, m);
        Ok(())
    }

    /// `ordered_columns`: `(a, b)`, each an `ordered_column` aliased `column`.
    fn ordered_columns(&mut self) -> R<()> {
        let m = self.a.mark();
        self.expect_punct(b"(", K::TLParen)?;
        loop {
            let c = self.a.mark();
            self.expect_ident(NONE, f::NAME)?;
            if self.is(K::KeywordAsc)? || self.is(K::KeywordDesc)? || self.at_punct(b"(")? {
                return self.refuse();
            }
            self.a.node(K::Column, c);
            if self.punct(b",", K::TComma)?.is_none() {
                break;
            }
        }
        self.expect_punct(b")", K::TRParen)?;
        self.a.node(K::OrderedColumns, m);
        Ok(())
    }

    /// `create [unique] index [concurrently] [[if not exists] name] on t [using m] (fields)`.
    fn create_index(&mut self) -> R<()> {
        let m = self.a.mark();
        self.expect(K::KeywordCreate)?;
        self.kw(K::KeywordUnique)?;
        self.expect(K::KeywordIndex)?;
        self.kw(K::KeywordConcurrently)?;
        if !self.is(K::KeywordOn)? {
            self.if_not_exists()?;
            let t = self.peek()?;
            if t.t == T::Str {
                return self.refuse();
            }
            self.expect_ident(Ctx::Sure(&[&[K::KeywordOn, K::KeywordIf]]), f::COLUMN)?;
        }
        self.expect(K::KeywordOn)?;
        if self.is(K::KeywordOnly)? {
            return self.refuse();
        }
        if self.object_reference(NONE, 0)?.is_none() {
            return self.refuse();
        }
        if self.kw(K::KeywordUsing)?.is_some() {
            let ws = self.words()?;
            let methods = [K::KeywordBtree, K::KeywordHash, K::KeywordGist, K::KeywordSpgist, K::KeywordGin, K::KeywordBrin];
            if let Some(&(_, k)) = ws.iter().find(|&&(_, k)| methods.contains(&k)) {
                self.expect(k)?;
            } else {
                self.expect_ident(NONE, f::ACCESS_METHOD)?;
            }
        }
        // `index_fields`.
        let n = self.a.mark();
        self.expect_punct(b"(", K::TLParen)?;
        loop {
            let fm = self.a.mark();
            if self.at_punct(b"(")? {
                return self.refuse();
            }
            let t = self.peek()?;
            if t.t == T::Str {
                return self.refuse();
            }
            // A column, or an invocation (`function:`).
            let save = (self.at, self.last_end, self.look);
            self.take(t);
            let call = self.at_punct(b"(")? || self.at_punct(b".")?;
            (self.at, self.last_end, self.look) = save;
            if call {
                return self.refuse();
            }
            self.expect_ident(NONE, f::COLUMN)?;
            if self.is(K::KeywordCollate)? {
                return self.refuse();
            }
            if self.kw(K::KeywordAsc)?.is_some() || self.kw(K::KeywordDesc)?.is_some() {
                let d = self.a.mark() - 1;
                self.a.node(K::Direction, d);
            } else if self.peek()?.t == T::Word {
                // An operator class, or `nulls`.
                return self.refuse();
            }
            if self.is(K::KeywordNulls)? {
                return self.refuse();
            }
            self.a.node(K::Field, fm);
            if self.punct(b",", K::TComma)?.is_none() {
                break;
            }
        }
        self.expect_punct(b")", K::TRParen)?;
        self.a.node(K::IndexFields, n);
        if self.is(K::KeywordInclude)? || self.is(K::KeywordNulls)? || self.is(K::KeywordWith)? || self.is(K::KeywordTablespace)? {
            return self.refuse();
        }
        if self.is(K::KeywordWhere)? {
            self.where_()?;
        }
        self.a.node(K::CreateIndex, m);
        Ok(())
    }

    /// `create [or replace] function f(arguments) [returns type] options and body`.
    fn create_function(&mut self) -> R<()> {
        let m = self.a.mark();
        self.expect(K::KeywordCreate)?;
        if self.kw(K::KeywordOr)?.is_some() {
            self.expect(K::KeywordReplace)?;
        }
        self.expect(K::KeywordFunction)?;
        if self.object_reference(NONE, 0)?.is_none() {
            return self.refuse();
        }
        self.function_arguments()?;
        if self.is(K::KeywordReturns)? {
            // `returns null on null input` is an option, not a return type.
            let save = (self.at, self.last_end, self.look);
            let t = self.peek()?;
            self.take(t);
            let strictness = self.is(K::KeywordNull)?;
            (self.at, self.last_end, self.look) = save;
            if !strictness {
                self.expect(K::KeywordReturns)?;
                if self.kw(K::KeywordSetof)?.is_some() {
                    self.type_(0)?;
                } else if self.kw(K::KeywordTable)?.is_some() {
                    self.table_elements()?;
                } else if self.kw(K::KeywordTrigger)?.is_none() {
                    self.type_(0)?;
                }
            }
        }
        let mut body = false;
        loop {
            if self.function_option()? {
                continue;
            }
            if !body && self.is(K::KeywordAs)? {
                let b = self.a.mark();
                self.expect(K::KeywordAs)?;
                self.function_string()?;
                if self.punct(b",", K::TComma)?.is_some() {
                    self.function_string()?;
                }
                self.a.node(K::FunctionBody, b);
                body = true;
                continue;
            }
            if !body && (self.is(K::KeywordReturn)? || self.is(K::KeywordBegin)?) {
                return self.refuse();
            }
            break;
        }
        if !body {
            return self.refuse();
        }
        self.a.node(K::CreateFunction, m);
        Ok(())
    }

    /// A function body's string: `@literal{_literal_string}`.
    fn function_string(&mut self) -> R<()> {
        let t = self.peek()?;
        match t.t {
            T::Str => self.string_literal(0),
            T::EStr | T::Dollar => {
                self.atom(false)?;
                Ok(())
            }
            _ => self.refuse(),
        }
    }

    /// `function_arguments`: `(argument, …)`, each `[mode] [name] type [default x]`.
    fn function_arguments(&mut self) -> R<()> {
        let m = self.a.mark();
        self.expect_punct(b"(", K::TLParen)?;
        if !self.at_punct(b")")? {
            loop {
                let a = self.a.mark();
                // `_argmode`.
                if self.kw(K::KeywordIn)?.is_some() {
                    self.kw(K::KeywordOut)?;
                } else if self.kw(K::KeywordOut)?.is_none() && self.kw(K::KeywordInout)?.is_none() {
                    self.kw(K::KeywordVariadic)?;
                }
                // `_param_name`: a word a type follows (a keyword among the grammar's names read as
                // an identifier).
                let t = self.peek()?;
                if t.t == T::Word || t.t == T::Quoted {
                    let ws = self.words()?;
                    let type_kw = ws.iter().any(|&(_, k)| kinds::FIRST_TYPE.contains(&k));
                    let named_kw = ws.iter().any(|&(_, k)| PARAM_NAMES.contains(&k));
                    if !type_kw || named_kw {
                        let save = (self.at, self.last_end, self.look);
                        self.take(t);
                        let n = self.peek()?;
                        let follows = (n.t == T::Word && !self.is(K::KeywordDefault)?) || n.t == T::Quoted;
                        let mode = self.is(K::KeywordIn)? || self.is(K::KeywordOut)? || self.is(K::KeywordInout)? || self.is(K::KeywordVariadic)?;
                        (self.at, self.last_end, self.look) = save;
                        if mode {
                            return self.refuse();
                        }
                        if follows {
                            self.take(t);
                            self.a.leaf(K::Identifier, 0, t.s, t.e);
                        }
                    }
                }
                self.type_(0)?;
                if self.kw(K::KeywordDefault)?.is_some() || self.punct(b"=", K::TEq)?.is_some() {
                    self.expr(0, 0)?;
                }
                self.a.node(K::FunctionArgument, a);
                if self.punct(b",", K::TComma)?.is_none() {
                    break;
                }
            }
        }
        self.expect_punct(b")", K::TRParen)?;
        self.a.node(K::FunctionArguments, m);
        Ok(())
    }

    /// One of a function's options, where one is next: its language, volatility, safety,
    /// strictness, cost, rows, security.
    fn function_option(&mut self) -> R<bool> {
        let ws = self.words()?;
        let Some(&(_, k)) = ws.first() else { return Ok(false) };
        let m = self.a.mark();
        let node = match k {
            K::KeywordLanguage => {
                self.expect(k)?;
                let t = self.peek()?;
                if t.t == T::Str {
                    self.take(t);
                    self.a.leaf(K::Literal, 0, t.s, t.e);
                } else {
                    self.expect_ident(NONE, 0)?;
                }
                K::FunctionLanguage
            }
            K::KeywordImmutable | K::KeywordStable | K::KeywordVolatile => {
                self.expect(k)?;
                K::FunctionVolatility
            }
            K::KeywordParallel => {
                self.expect(k)?;
                if self.kw(K::KeywordSafe)?.is_none() && self.kw(K::KeywordUnsafe)?.is_none() {
                    self.expect(K::KeywordRestricted)?;
                }
                K::FunctionSafety
            }
            K::KeywordStrict => {
                self.expect(k)?;
                K::FunctionStrictness
            }
            K::KeywordCalled | K::KeywordReturns => {
                if k == K::KeywordCalled {
                    self.expect(k)?;
                } else {
                    self.expect(K::KeywordReturns)?;
                    self.expect(K::KeywordNull)?;
                }
                self.expect(K::KeywordOn)?;
                self.expect(K::KeywordNull)?;
                self.expect(K::KeywordInput)?;
                K::FunctionStrictness
            }
            K::KeywordLeakproof | K::KeywordNot => {
                self.kw(K::KeywordNot)?;
                self.expect(K::KeywordLeakproof)?;
                K::FunctionLeakproof
            }
            K::KeywordSecurity | K::KeywordExternal => {
                self.kw(K::KeywordExternal)?;
                self.expect(K::KeywordSecurity)?;
                if self.kw(K::KeywordInvoker)?.is_none() {
                    self.expect(K::KeywordDefiner)?;
                }
                K::FunctionSecurity
            }
            K::KeywordCost | K::KeywordRows => {
                self.expect(k)?;
                let t = self.peek()?;
                if t.t != T::Num || !self.src[t.s..t.e].iter().all(u8::is_ascii_digit) {
                    return self.refuse();
                }
                self.take(t);
                if k == K::KeywordCost { K::FunctionCost } else { K::FunctionRows }
            }
            K::KeywordWindow | K::KeywordSupport | K::KeywordSet | K::KeywordTransform => return self.refuse(),
            _ => return Ok(false),
        };
        self.a.node(node, m);
        Ok(true)
    }

    /// `create extension [if not exists] name [with] [schema s | version v | cascade]…`.
    fn create_extension(&mut self) -> R<()> {
        let m = self.a.mark();
        self.expect(K::KeywordCreate)?;
        self.expect(K::KeywordExtension)?;
        self.if_not_exists()?;
        self.expect_ident(Ctx::Sure(&[&[K::KeywordIf]]), 0)?;
        self.kw(K::KeywordWith)?;
        loop {
            if self.kw(K::KeywordSchema)?.is_some() {
                self.expect_ident(NONE, 0)?;
            } else if self.kw(K::KeywordVersion)?.is_some() {
                let t = self.peek()?;
                if t.t == T::Str {
                    self.take(t);
                    self.a.leaf(K::Literal, 0, t.s, t.e);
                } else {
                    self.expect_ident(NONE, 0)?;
                }
            } else if self.kw(K::KeywordCascade)?.is_none() {
                break;
            }
        }
        self.a.node(K::CreateExtension, m);
        Ok(())
    }

    /// `create schema [if not exists] name [authorization role]`.
    fn create_schema(&mut self) -> R<()> {
        let m = self.a.mark();
        self.expect(K::KeywordCreate)?;
        self.expect(K::KeywordSchema)?;
        if self.kw(K::KeywordAuthorization)?.is_some() {
            self.expect_ident(NONE, 0)?;
        } else {
            self.if_not_exists()?;
            self.expect_ident(Ctx::Sure(&[&[K::KeywordIf, K::KeywordAuthorization]]), 0)?;
            if self.kw(K::KeywordAuthorization)?.is_some() {
                self.expect_ident(NONE, 0)?;
            }
        }
        if self.is(K::KeywordCreate)? {
            return self.refuse();
        }
        self.a.node(K::CreateSchema, m);
        Ok(())
    }

    /// `create [or replace] [temporary] view v [(columns)] as query`.
    fn create_view(&mut self) -> R<()> {
        let m = self.a.mark();
        self.expect(K::KeywordCreate)?;
        if self.kw(K::KeywordOr)?.is_some() {
            self.expect(K::KeywordReplace)?;
        }
        if self.kw(K::KeywordTemp)?.is_none() {
            self.kw(K::KeywordTemporary)?;
        }
        if self.is(K::KeywordRecursive)? {
            return self.refuse();
        }
        self.expect(K::KeywordView)?;
        self.if_not_exists()?;
        if self.object_reference(Ctx::Sure(&[&[K::KeywordIf]]), 0)?.is_none() {
            return self.refuse();
        }
        if self.punct(b"(", K::TLParen)?.is_some() {
            loop {
                self.expect_ident(NONE, 0)?;
                if self.punct(b",", K::TComma)?.is_none() {
                    break;
                }
            }
            self.expect_punct(b")", K::TRParen)?;
        }
        if self.is(K::KeywordWith)? {
            return self.refuse();
        }
        self.expect(K::KeywordAs)?;
        let q = self.a.mark();
        self.dml_read()?;
        self.a.node(K::CreateQuery, q);
        if self.is(K::KeywordWith)? {
            return self.refuse();
        }
        self.a.node(K::CreateView, m);
        Ok(())
    }

    /// `drop table | view | index …`.
    fn drop(&mut self) -> R<()> {
        let m = self.a.mark();
        let save = (self.at, self.last_end, self.look);
        let t = self.peek()?;
        self.take(t);
        let next: Vec<K> = self.words()?.iter().map(|&(_, k)| k).collect();
        (self.at, self.last_end, self.look) = save;
        let (k, node) = if next.contains(&K::KeywordTable) {
            (K::KeywordTable, K::DropTable)
        } else if next.contains(&K::KeywordView) {
            (K::KeywordView, K::DropView)
        } else if next.contains(&K::KeywordIndex) {
            (K::KeywordIndex, K::DropIndex)
        } else if next.contains(&K::KeywordFunction) {
            (K::KeywordFunction, K::DropFunction)
        } else {
            return self.refuse();
        };
        self.expect(K::KeywordDrop)?;
        self.expect(k)?;
        if node == K::DropIndex {
            self.kw(K::KeywordConcurrently)?;
            self.if_exists()?;
            self.expect_ident(Ctx::Sure(&[&[K::KeywordIf]]), f::NAME)?;
            if self.at_punct(b".")? || self.at_punct(b",")? {
                return self.refuse();
            }
        } else {
            self.if_exists()?;
            loop {
                if self.object_reference(Ctx::Sure(&[&[K::KeywordIf]]), 0)?.is_none() {
                    return self.refuse();
                }
                if node == K::DropFunction && self.at_punct(b"(")? {
                    self.function_arguments()?;
                }
                if self.punct(b",", K::TComma)?.is_none() {
                    break;
                }
            }
        }
        if self.kw(K::KeywordCascade)?.is_none() {
            self.kw(K::KeywordRestrict)?;
        }
        if node == K::DropIndex && self.is(K::KeywordOn)? {
            return self.refuse();
        }
        self.a.node(node, m);
        Ok(())
    }

    /// `alter table [if exists] t add [column] …, drop column …`: the common forms; others refused.
    fn alter(&mut self) -> R<()> {
        let m = self.a.mark();
        self.expect(K::KeywordAlter)?;
        if !self.is(K::KeywordTable)? {
            return self.refuse();
        }
        self.expect(K::KeywordTable)?;
        self.if_exists()?;
        if self.is(K::KeywordOnly)? {
            return self.refuse();
        }
        if self.object_reference(Ctx::Sure(&[&[K::KeywordIf, K::KeywordOnly]]), 0)?.is_none() {
            return self.refuse();
        }
        loop {
            let s = self.a.mark();
            if self.is(K::KeywordAdd)? {
                self.expect(K::KeywordAdd)?;
                let ws = self.words()?;
                if ws.iter().any(|&(_, k)| matches!(k, K::KeywordConstraint | K::KeywordPrimary | K::KeywordUnique | K::KeywordForeign | K::KeywordCheck | K::KeywordExclude)) {
                    return self.refuse();
                }
                self.kw(K::KeywordColumn)?;
                self.if_not_exists()?;
                self.column_definition()?;
                if self.is(K::KeywordFirst)? || self.is(K::KeywordAfter)? {
                    return self.refuse();
                }
                self.a.node(K::AddColumn, s);
            } else if self.is(K::KeywordDrop)? {
                self.expect(K::KeywordDrop)?;
                if !self.is(K::KeywordColumn)? {
                    return self.refuse();
                }
                self.expect(K::KeywordColumn)?;
                self.if_exists()?;
                self.expect_ident(Ctx::Sure(&[&[K::KeywordIf]]), f::NAME)?;
                if self.kw(K::KeywordCascade)?.is_none() {
                    self.kw(K::KeywordRestrict)?;
                }
                self.a.node(K::DropColumn, s);
            } else {
                return self.refuse();
            }
            if self.punct(b",", K::TComma)?.is_none() {
                break;
            }
        }
        self.a.node(K::AlterTable, m);
        Ok(())
    }

    /// `set [session | local] name to | = value, …`, `set time zone …`.
    fn set_statement(&mut self) -> R<()> {
        let m = self.a.mark();
        self.expect(K::KeywordSet)?;
        if self.kw(K::KeywordSession)?.is_none() {
            self.kw(K::KeywordLocal)?;
        }
        let ws = self.words()?;
        if ws.iter().any(|&(_, k)| matches!(k, K::KeywordSchema | K::KeywordNames | K::KeywordTime | K::KeywordRole | K::KeywordTransaction | K::KeywordConstraints | K::KeywordAuthorization | K::KeywordCharacteristics)) {
            return self.refuse();
        }
        loop {
            if self.object_reference(Ctx::Sure(&[&[K::KeywordSchema, K::KeywordNames, K::KeywordTime, K::KeywordRole, K::KeywordTransaction, K::KeywordSession, K::KeywordLocal, K::KeywordConstraints]]), 0)?.is_none() {
                return self.refuse();
            }
            if self.kw(K::KeywordTo)?.is_none() {
                if self.is(K::KeywordFrom)? {
                    return self.refuse();
                }
                self.expect_punct(b"=", K::TEq)?;
            }
            loop {
                self.set_value()?;
                if self.punct(b",", K::TComma)?.is_none() {
                    break;
                }
                // `,` then the next name and its `to`/`=`, or the next value.
                let save = (self.at, self.last_end, self.look);
                let t = self.peek()?;
                self.take(t);
                let assignment = self.is(K::KeywordTo)? || self.at_punct(b"=")? || self.at_punct(b".")?;
                (self.at, self.last_end, self.look) = save;
                if assignment {
                    return self.refuse();
                }
            }
            break;
        }
        self.a.node(K::SetStatement, m);
        Ok(())
    }

    /// `_set_value`: a literal, `default`, `on`, `off`, or an identifier.
    fn set_value(&mut self) -> R<()> {
        let t = self.peek()?;
        match t.t {
            T::Str | T::EStr | T::Num | T::Dollar => {
                self.atom(false)?;
            }
            T::Punct if matches!(self.byte(t.s), b'-' | b'+') => {
                self.atom(false)?;
            }
            T::Word | T::Quoted => {
                let ws = self.words()?;
                if let Some(&(_, k)) = ws.iter().find(|&&(_, k)| matches!(k, K::KeywordDefault | K::KeywordOn | K::KeywordOff)) {
                    self.expect(k)?;
                } else if ws.iter().any(|&(_, k)| matches!(k, K::KeywordTrue | K::KeywordFalse | K::KeywordNull)) {
                    self.atom(false)?;
                } else {
                    self.expect_ident(NONE, 0)?;
                }
            }
            _ => return self.refuse(),
        }
        Ok(())
    }

    /// `reset name | all`.
    fn reset_statement(&mut self) -> R<()> {
        let m = self.a.mark();
        self.expect(K::KeywordReset)?;
        if self.kw(K::KeywordAll)?.is_none() {
            if self.is(K::KeywordSession)? || self.is(K::KeywordRole)? {
                return self.refuse();
            }
            if self.object_reference(Ctx::Sure(&[&[K::KeywordAll, K::KeywordSession, K::KeywordRole]]), 0)?.is_none() {
                return self.refuse();
            }
        }
        self.a.node(K::ResetStatement, m);
        Ok(())
    }

    /// `comment on table | view | column | index | schema … is 'text' | null`.
    fn comment_statement(&mut self) -> R<()> {
        let m = self.a.mark();
        self.expect(K::KeywordComment)?;
        self.expect(K::KeywordOn)?;
        let ws = self.words()?;
        let Some(&(_, k)) = ws.first() else { return self.refuse() };
        match k {
            K::KeywordTable | K::KeywordView | K::KeywordIndex | K::KeywordSequence | K::KeywordExtension => {
                self.expect(k)?;
                if self.object_reference(NONE, 0)?.is_none() {
                    return self.refuse();
                }
            }
            K::KeywordSchema | K::KeywordDatabase | K::KeywordRole | K::KeywordType | K::KeywordTablespace => {
                self.expect(k)?;
                self.expect_ident(NONE, 0)?;
            }
            K::KeywordColumn => {
                self.expect(K::KeywordColumn)?;
                // `@object_reference{_qualified_field}`.
                let n = self.a.mark();
                let first = self.expect_ident(NONE, f::NAME)?;
                let mut parts = vec![first];
                while self.at_punct(b".")? {
                    let t = self.peek()?;
                    self.take(Tok { e: t.s + 1, ..t });
                    parts.push(self.expect_ident(NONE, f::NAME)?);
                }
                self.a.stack.truncate(n);
                match parts.len() {
                    2 | 3 => {
                        let (head, name) = parts.split_at(parts.len() - 1);
                        self.push_reference(head, 0);
                        let d = self.dot_before(name[0]);
                        self.a.leaf(K::TDot, 0, d, d + 1);
                        self.a.stack.push(name[0]);
                    }
                    _ => return self.refuse(),
                }
                self.a.node(K::ObjectReference, n);
            }
            _ => return self.refuse(),
        }
        self.expect(K::KeywordIs)?;
        let t = self.peek()?;
        if self.kw(K::KeywordNull)?.is_none() {
            if !matches!(t.t, T::Str | T::EStr | T::Dollar) {
                return self.refuse();
            }
            if t.t == T::Str {
                self.string_literal(0)?;
            } else {
                self.atom(false)?;
            }
        }
        self.a.node(K::CommentStatement, m);
        Ok(())
    }
}
