use std::{convert, fmt, mem, ptr, result, str};
use std::ffi::CStr;
use std::os::raw::{c_char, c_int, c_void};
use std::slice::from_raw_parts;

use super::ffi;
use super::{Connection, RawStatement, Result, Error, ValueRef, Row, Rows, AndThenRows, MappedRows};
use super::str_to_cstring;
use types::{ToSql, ToSqlOutput};

/// A prepared statement.
pub struct Statement<'conn> {
    conn: &'conn Connection,
    stmt: RawStatement,
}

impl<'conn> Statement<'conn> {
    /// Get all the column names in the result set of the prepared statement.
    pub fn column_names(&self) -> Vec<&str> {
        let n = self.column_count();
        let mut cols = Vec::with_capacity(n as usize);
        for i in 0..n {
            let slice = self.stmt.column_name(i);
            let s = str::from_utf8(slice.to_bytes()).unwrap();
            cols.push(s);
        }
        cols
    }

    /// Return the number of columns in the result set returned by the prepared statement.
    pub fn column_count(&self) -> i32 {
        self.stmt.column_count()
    }

    /// Returns the column index in the result set for a given column name.
    ///
    /// If there is no AS clause then the name of the column is unspecified and may change from one
    /// release of SQLite to the next.
    ///
    /// # Failure
    ///
    /// Will return an `Error::InvalidColumnName` when there is no column with the specified `name`.
    pub fn column_index(&self, name: &str) -> Result<i32> {
        let bytes = name.as_bytes();
        let n = self.column_count();
        for i in 0..n {
            if bytes == self.stmt.column_name(i).to_bytes() {
                return Ok(i);
            }
        }
        Err(Error::InvalidColumnName(String::from(name)))
    }

    /// Execute the prepared statement.
    ///
    /// On success, returns the number of rows that were changed or inserted or deleted (via
    /// `sqlite3_changes`).
    ///
    /// ## Example
    ///
    /// ```rust,no_run
    /// # use rusqlite::{Connection, Result};
    /// fn update_rows(conn: &Connection) -> Result<()> {
    ///     let mut stmt = try!(conn.prepare("UPDATE foo SET bar = 'baz' WHERE qux = ?"));
    ///
    ///     try!(stmt.execute(&[&1i32]));
    ///     try!(stmt.execute(&[&2i32]));
    ///
    ///     Ok(())
    /// }
    /// ```
    ///
    /// # Failure
    ///
    /// Will return `Err` if binding parameters fails, the executed statement returns rows (in
    /// which case `query` should be used instead), or the underling SQLite call fails.
    pub fn execute(&mut self, params: &[&ToSql]) -> Result<c_int> {
        try!(self.bind_parameters(params));
        self.execute_with_bound_parameters()
    }

    /// Execute the prepared statement with named parameter(s). If any parameters
    /// that were in the prepared statement are not included in `params`, they
    /// will continue to use the most-recently bound value from a previous call
    /// to `execute_named`, or `NULL` if they have never been bound.
    ///
    /// On success, returns the number of rows that were changed or inserted or deleted (via
    /// `sqlite3_changes`).
    ///
    /// ## Example
    ///
    /// ```rust,no_run
    /// # use rusqlite::{Connection, Result};
    /// fn insert(conn: &Connection) -> Result<i32> {
    ///     let mut stmt = try!(conn.prepare("INSERT INTO test (name) VALUES (:name)"));
    ///     stmt.execute_named(&[(":name", &"one")])
    /// }
    /// ```
    ///
    /// # Failure
    ///
    /// Will return `Err` if binding parameters fails, the executed statement returns rows (in
    /// which case `query` should be used instead), or the underling SQLite call fails.
    pub fn execute_named(&mut self, params: &[(&str, &ToSql)]) -> Result<c_int> {
        try!(self.bind_parameters_named(params));
        self.execute_with_bound_parameters()
    }

    /// Execute the prepared statement, returning a handle to the resulting rows.
    ///
    /// Due to lifetime restricts, the rows handle returned by `query` does not
    /// implement the `Iterator` trait. Consider using `query_map` or `query_and_then`
    /// instead, which do.
    ///
    /// ## Example
    ///
    /// ```rust,no_run
    /// # use rusqlite::{Connection, Result};
    /// fn get_names(conn: &Connection) -> Result<Vec<String>> {
    ///     let mut stmt = try!(conn.prepare("SELECT name FROM people"));
    ///     let mut rows = try!(stmt.query(&[]));
    ///
    ///     let mut names = Vec::new();
    ///     while let Some(result_row) = rows.next() {
    ///         let row = try!(result_row);
    ///         names.push(row.get(0));
    ///     }
    ///
    ///     Ok(names)
    /// }
    /// ```
    ///
    /// ## Failure
    ///
    /// Will return `Err` if binding parameters fails.
    pub fn query<'a>(&'a mut self, params: &[&ToSql]) -> Result<Rows<'a>> {
        try!(self.bind_parameters(params));
        Ok(Rows::new(self))
    }

    /// Execute the prepared statement with named parameter(s), returning a handle for the
    /// resulting rows. If any parameters that were in the prepared statement are not included in
    /// `params`, they will continue to use the most-recently bound value from a previous call to
    /// `query_named`, or `NULL` if they have never been bound.
    ///
    /// ## Example
    ///
    /// ```rust,no_run
    /// # use rusqlite::{Connection, Result};
    /// fn query(conn: &Connection) -> Result<()> {
    ///     let mut stmt = try!(conn.prepare("SELECT * FROM test where name = :name"));
    ///     let mut rows = try!(stmt.query_named(&[(":name", &"one")]));
    ///     while let Some(row) = rows.next() {
    ///         // ...
    ///     }
    ///     Ok(())
    /// }
    /// ```
    ///
    /// # Failure
    ///
    /// Will return `Err` if binding parameters fails.
    pub fn query_named<'a>(&'a mut self, params: &[(&str, &ToSql)]) -> Result<Rows<'a>> {
        try!(self.bind_parameters_named(params));
        Ok(Rows::new(self))
    }

    /// Executes the prepared statement and maps a function over the resulting rows, returning
    /// an iterator over the mapped function results.
    ///
    /// ## Example
    ///
    /// ```rust,no_run
    /// # use rusqlite::{Connection, Result};
    /// fn get_names(conn: &Connection) -> Result<Vec<String>> {
    ///     let mut stmt = try!(conn.prepare("SELECT name FROM people"));
    ///     let rows = try!(stmt.query_map(&[], |row| row.get(0)));
    ///
    ///     let mut names = Vec::new();
    ///     for name_result in rows {
    ///         names.push(try!(name_result));
    ///     }
    ///
    ///     Ok(names)
    /// }
    /// ```
    ///
    /// ## Failure
    ///
    /// Will return `Err` if binding parameters fails.
    pub fn query_map<'a, T, F>(&'a mut self, params: &[&ToSql], f: F) -> Result<MappedRows<'a, F>>
        where F: FnMut(&Row) -> T
    {
        let row_iter = try!(self.query(params));

        Ok(MappedRows {
            rows: row_iter,
            map: f,
        })
    }

    /// Execute the prepared statement with named parameter(s), returning an iterator over the
    /// result of calling the mapping function over the query's rows. If any parameters that were
    /// in the prepared statement are not included in `params`, they will continue to use the
    /// most-recently bound value from a previous call to `query_named`, or `NULL` if they have
    /// never been bound.
    ///
    /// ## Example
    ///
    /// ```rust,no_run
    /// # use rusqlite::{Connection, Result};
    /// fn get_names(conn: &Connection) -> Result<Vec<String>> {
    ///     let mut stmt = try!(conn.prepare("SELECT name FROM people WHERE id = :id"));
    ///     let rows = try!(stmt.query_map_named(&[(":id", &"one")], |row| row.get(0)));
    ///
    ///     let mut names = Vec::new();
    ///     for name_result in rows {
    ///         names.push(try!(name_result));
    ///     }
    ///
    ///     Ok(names)
    /// }
    /// ```
    ///
    /// ## Failure
    ///
    /// Will return `Err` if binding parameters fails.
    pub fn query_map_named<'a, T, F>(&'a mut self,
                                     params: &[(&str, &ToSql)],
                                     f: F)
                                     -> Result<MappedRows<'a, F>>
        where F: FnMut(&Row) -> T
    {
        let rows = try!(self.query_named(params));
        Ok(MappedRows {
            rows: rows,
            map: f,
        })
    }

    /// Executes the prepared statement and maps a function over the resulting
    /// rows, where the function returns a `Result` with `Error` type implementing
    /// `std::convert::From<Error>` (so errors can be unified).
    ///
    /// # Failure
    ///
    /// Will return `Err` if binding parameters fails.
    pub fn query_and_then<'a, T, E, F>(&'a mut self,
                                       params: &[&ToSql],
                                       f: F)
                                       -> Result<AndThenRows<'a, F>>
        where E: convert::From<Error>,
              F: FnMut(&Row) -> result::Result<T, E>
    {
        let row_iter = try!(self.query(params));

        Ok(AndThenRows {
            rows: row_iter,
            map: f,
        })
    }

    /// Execute the prepared statement with named parameter(s), returning an iterator over the
    /// result of calling the mapping function over the query's rows. If any parameters that were
    /// in the prepared statement are not included in `params`, they will continue to use the
    /// most-recently bound value from a previous call to `query_named`, or `NULL` if they have
    /// never been bound.
    ///
    /// ## Example
    ///
    /// ```rust,no_run
    /// # use rusqlite::{Connection, Result};
    /// struct Person { name: String };
    ///
    /// fn name_to_person(name: String) -> Result<Person> {
    ///     // ... check for valid name
    ///     Ok(Person{ name: name })
    /// }
    ///
    /// fn get_names(conn: &Connection) -> Result<Vec<Person>> {
    ///     let mut stmt = try!(conn.prepare("SELECT name FROM people WHERE id = :id"));
    ///     let rows = try!(stmt.query_and_then_named(&[(":id", &"one")], |row| {
    ///         name_to_person(row.get(0))
    ///     }));
    ///
    ///     let mut persons = Vec::new();
    ///     for person_result in rows {
    ///         persons.push(try!(person_result));
    ///     }
    ///
    ///     Ok(persons)
    /// }
    /// ```
    ///
    /// ## Failure
    ///
    /// Will return `Err` if binding parameters fails.
    pub fn query_and_then_named<'a, T, E, F>(&'a mut self,
                                             params: &[(&str, &ToSql)],
                                             f: F)
                                             -> Result<AndThenRows<'a, F>>
        where E: convert::From<Error>,
              F: FnMut(&Row) -> result::Result<T, E>
    {
        let rows = try!(self.query_named(params));
        Ok(AndThenRows {
            rows: rows,
            map: f,
        })
    }

    /// Consumes the statement.
    ///
    /// Functionally equivalent to the `Drop` implementation, but allows callers to see any errors
    /// that occur.
    ///
    /// # Failure
    ///
    /// Will return `Err` if the underlying SQLite call fails.
    pub fn finalize(mut self) -> Result<()> {
        self.finalize_()
    }

    /// Return the index of an SQL parameter given its name.
    ///
    /// # Failure
    ///
    /// Will return Err if `name` is invalid. Will return Ok(None) if the name
    /// is valid but not a bound parameter of this statement.
    pub fn parameter_index(&self, name: &str) -> Result<Option<i32>> {
        let c_name = try!(str_to_cstring(name));
        Ok(self.stmt.bind_parameter_index(&c_name))
    }

    fn bind_parameters(&mut self, params: &[&ToSql]) -> Result<()> {
        assert!(params.len() as c_int == self.stmt.bind_parameter_count(),
                "incorrect number of parameters to query(): expected {}, got {}",
                self.stmt.bind_parameter_count(),
                params.len());

        for (i, p) in params.iter().enumerate() {
            try!(self.bind_parameter(*p, (i + 1) as c_int));
        }

        Ok(())
    }

    fn bind_parameters_named(&mut self, params: &[(&str, &ToSql)]) -> Result<()> {
        for &(name, value) in params {
            if let Some(i) = try!(self.parameter_index(name)) {
                try!(self.bind_parameter(value, i));
            } else {
                return Err(Error::InvalidParameterName(name.into()));
            }
        }
        Ok(())
    }

    fn bind_parameter(&self, param: &ToSql, col: c_int) -> Result<()> {
        let value = try!(param.to_sql());

        let ptr = unsafe { self.stmt.ptr() };
        let value = match value {
            ToSqlOutput::Borrowed(v) => v,
            ToSqlOutput::Owned(ref v) => ValueRef::from(v),

            #[cfg(feature = "blob")]
            ToSqlOutput::ZeroBlob(len) => {
                return self.conn
                    .decode_result(unsafe { ffi::sqlite3_bind_zeroblob(ptr, col, len) });
            }
        };
        self.conn.decode_result(match value {
            ValueRef::Null => unsafe { ffi::sqlite3_bind_null(ptr, col) },
            ValueRef::Integer(i) => unsafe { ffi::sqlite3_bind_int64(ptr, col, i) },
            ValueRef::Real(r) => unsafe { ffi::sqlite3_bind_double(ptr, col, r) },
            ValueRef::Text(s) => unsafe {
                let length = s.len();
                if length > ::std::i32::MAX as usize {
                    ffi::SQLITE_TOOBIG
                } else {
                    let c_str = try!(str_to_cstring(s));
                    let destructor = if length > 0 {
                        ffi::SQLITE_TRANSIENT()
                    } else {
                        ffi::SQLITE_STATIC()
                    };
                    ffi::sqlite3_bind_text(ptr, col, c_str.as_ptr(), length as c_int, destructor)
                }
            },
            ValueRef::Blob(b) => unsafe {
                let length = b.len();
                if length > ::std::i32::MAX as usize {
                    ffi::SQLITE_TOOBIG
                } else if length == 0 {
                    ffi::sqlite3_bind_zeroblob(ptr, col, 0)
                } else {
                    ffi::sqlite3_bind_blob(ptr,
                                           col,
                                           b.as_ptr() as *const c_void,
                                           length as c_int,
                                           ffi::SQLITE_TRANSIENT())
                }
            },
        })
    }

    fn execute_with_bound_parameters(&mut self) -> Result<c_int> {
        let r = self.stmt.step();
        self.stmt.reset();
        match r {
            ffi::SQLITE_DONE => {
                if self.column_count() == 0 {
                    Ok(self.conn.changes())
                } else {
                    Err(Error::ExecuteReturnedResults)
                }
            }
            ffi::SQLITE_ROW => Err(Error::ExecuteReturnedResults),
            _ => Err(self.conn.decode_result(r).unwrap_err()),
        }
    }

    fn finalize_(&mut self) -> Result<()> {
        let mut stmt = RawStatement::new(ptr::null_mut());
        mem::swap(&mut stmt, &mut self.stmt);
        self.conn.decode_result(stmt.finalize())
    }
}

impl<'conn> Into<RawStatement> for Statement<'conn> {
    fn into(mut self) -> RawStatement {
        let mut stmt = RawStatement::new(ptr::null_mut());
        mem::swap(&mut stmt, &mut self.stmt);
        stmt
    }
}

impl<'conn> fmt::Debug for Statement<'conn> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let sql = str::from_utf8(self.stmt.sql().to_bytes());
        f.debug_struct("Statement")
            .field("conn", self.conn)
            .field("stmt", &self.stmt)
            .field("sql", &sql)
            .finish()
    }
}

impl<'conn> Drop for Statement<'conn> {
    #[allow(unused_must_use)]
    fn drop(&mut self) {
        self.finalize_();
    }
}

// TODO: This trait lets us have "pub(crate)" visibility on some Statement methods. Remove this
// once pub(crate) is stable.
pub trait StatementCrateImpl<'conn> {
    fn new(conn: &'conn Connection, stmt: RawStatement) -> Self;
    fn last_insert_rowid(&self) -> i64;
    fn value_ref(&self, col: c_int) -> ValueRef;
    fn step(&self) -> Result<bool>;
    fn reset(&self) -> c_int;
}

impl<'conn> StatementCrateImpl<'conn> for Statement<'conn> {
    fn new(conn: &Connection, stmt: RawStatement) -> Statement {
        Statement {
            conn: conn,
            stmt: stmt,
        }
    }

    fn last_insert_rowid(&self) -> i64 {
        self.conn.last_insert_rowid()
    }

    fn value_ref(&self, col: c_int) -> ValueRef {
        let raw = unsafe { self.stmt.ptr() };

        match self.stmt.column_type(col) {
            ffi::SQLITE_NULL => ValueRef::Null,
            ffi::SQLITE_INTEGER => {
                ValueRef::Integer(unsafe { ffi::sqlite3_column_int64(raw, col) })
            }
            ffi::SQLITE_FLOAT => ValueRef::Real(unsafe { ffi::sqlite3_column_double(raw, col) }),
            ffi::SQLITE_TEXT => {
                let s = unsafe {
                    let text = ffi::sqlite3_column_text(raw, col);
                    assert!(!text.is_null(),
                            "unexpected SQLITE_TEXT column type with NULL data");
                    CStr::from_ptr(text as *const c_char)
                };

                // sqlite3_column_text returns UTF8 data, so our unwrap here should be fine.
                let s = s.to_str().expect("sqlite3_column_text returned invalid UTF-8");
                ValueRef::Text(s)
            }
            ffi::SQLITE_BLOB => {
                let (blob, len) = unsafe {
                    (ffi::sqlite3_column_blob(raw, col), ffi::sqlite3_column_bytes(raw, col))
                };

                assert!(len >= 0,
                        "unexpected negative return from sqlite3_column_bytes");
                if len > 0 {
                    assert!(!blob.is_null(),
                            "unexpected SQLITE_BLOB column type with NULL data");
                    ValueRef::Blob(unsafe { from_raw_parts(blob as *const u8, len as usize) })
                } else {
                    // The return value from sqlite3_column_blob() for a zero-length BLOB
                    // is a NULL pointer.
                    ValueRef::Blob(&[])
                }
            }
            _ => unreachable!("sqlite3_column_type returned invalid value"),
        }
    }

    fn step(&self) -> Result<bool> {
        match self.stmt.step() {
            ffi::SQLITE_ROW => Ok(true),
            ffi::SQLITE_DONE => Ok(false),
            code => Err(self.conn.decode_result(code).unwrap_err()),
        }
    }

    fn reset(&self) -> c_int {
        self.stmt.reset()
    }
}

#[cfg(test)]
mod test {
    use Connection;
    use error::Error;

    #[test]
    fn test_execute_named() {
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch("CREATE TABLE foo(x INTEGER)").unwrap();

        assert_eq!(db.execute_named("INSERT INTO foo(x) VALUES (:x)", &[(":x", &1i32)]).unwrap(),
                   1);
        assert_eq!(db.execute_named("INSERT INTO foo(x) VALUES (:x)", &[(":x", &2i32)]).unwrap(),
                   1);

        assert_eq!(3i32,
                   db.query_row_named::<i32, _>("SELECT SUM(x) FROM foo WHERE x > :x",
                                                  &[(":x", &0i32)],
                                                  |r| r.get(0))
                       .unwrap());
    }

    #[test]
    fn test_stmt_execute_named() {
        let db = Connection::open_in_memory().unwrap();
        let sql = "CREATE TABLE test (id INTEGER PRIMARY KEY NOT NULL, name TEXT NOT NULL, flag \
                   INTEGER)";
        db.execute_batch(sql).unwrap();

        let mut stmt = db.prepare("INSERT INTO test (name) VALUES (:name)").unwrap();
        stmt.execute_named(&[(":name", &"one")]).unwrap();

        assert_eq!(1i32,
                   db.query_row_named::<i32, _>("SELECT COUNT(*) FROM test WHERE name = :name",
                                                  &[(":name", &"one")],
                                                  |r| r.get(0))
                       .unwrap());
    }

    #[test]
    fn test_query_named() {
        let db = Connection::open_in_memory().unwrap();
        let sql = r#"
        CREATE TABLE test (id INTEGER PRIMARY KEY NOT NULL, name TEXT NOT NULL, flag INTEGER);
        INSERT INTO test(id, name) VALUES (1, "one");
        "#;
        db.execute_batch(sql).unwrap();

        let mut stmt = db.prepare("SELECT id FROM test where name = :name").unwrap();
        let mut rows = stmt.query_named(&[(":name", &"one")]).unwrap();

        let id: i32 = rows.next().unwrap().unwrap().get(0);
        assert_eq!(1, id);
    }

    #[test]
    fn test_query_map_named() {
        let db = Connection::open_in_memory().unwrap();
        let sql = r#"
        CREATE TABLE test (id INTEGER PRIMARY KEY NOT NULL, name TEXT NOT NULL, flag INTEGER);
        INSERT INTO test(id, name) VALUES (1, "one");
        "#;
        db.execute_batch(sql).unwrap();

        let mut stmt = db.prepare("SELECT id FROM test where name = :name").unwrap();
        let mut rows = stmt.query_map_named(&[(":name", &"one")], |row| {
                let id: i32 = row.get(0);
                2 * id
            })
            .unwrap();

        let doubled_id: i32 = rows.next().unwrap().unwrap();
        assert_eq!(2, doubled_id);
    }

    #[test]
    fn test_query_and_then_named() {

        let db = Connection::open_in_memory().unwrap();
        let sql = r#"
        CREATE TABLE test (id INTEGER PRIMARY KEY NOT NULL, name TEXT NOT NULL, flag INTEGER);
        INSERT INTO test(id, name) VALUES (1, "one");
        INSERT INTO test(id, name) VALUES (2, "one");
        "#;
        db.execute_batch(sql).unwrap();

        let mut stmt = db.prepare("SELECT id FROM test where name = :name ORDER BY id ASC")
            .unwrap();
        let mut rows = stmt.query_and_then_named(&[(":name", &"one")], |row| {
                let id: i32 = row.get(0);
                if id == 1 {
                    Ok(id)
                } else {
                    Err(Error::SqliteSingleThreadedMode)
                }
            })
            .unwrap();

        // first row should be Ok
        let doubled_id: i32 = rows.next().unwrap().unwrap();
        assert_eq!(1, doubled_id);

        // second row should be Err
        match rows.next().unwrap() {
            Ok(_) => panic!("invalid Ok"),
            Err(Error::SqliteSingleThreadedMode) => (),
            Err(_) => panic!("invalid Err"),
        }
    }

    #[test]
    fn test_unbound_parameters_are_null() {
        let db = Connection::open_in_memory().unwrap();
        let sql = "CREATE TABLE test (x TEXT, y TEXT)";
        db.execute_batch(sql).unwrap();

        let mut stmt = db.prepare("INSERT INTO test (x, y) VALUES (:x, :y)").unwrap();
        stmt.execute_named(&[(":x", &"one")]).unwrap();

        let result: Option<String> =
            db.query_row("SELECT y FROM test WHERE x = 'one'", &[], |row| row.get(0))
                .unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_unbound_parameters_are_reused() {
        let db = Connection::open_in_memory().unwrap();
        let sql = "CREATE TABLE test (x TEXT, y TEXT)";
        db.execute_batch(sql).unwrap();

        let mut stmt = db.prepare("INSERT INTO test (x, y) VALUES (:x, :y)").unwrap();
        stmt.execute_named(&[(":x", &"one")]).unwrap();
        stmt.execute_named(&[(":y", &"two")]).unwrap();

        let result: String =
            db.query_row("SELECT x FROM test WHERE y = 'two'", &[], |row| row.get(0))
                .unwrap();
        assert_eq!(result, "one");
    }
}
