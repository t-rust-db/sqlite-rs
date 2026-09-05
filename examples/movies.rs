// Copyright 2026 Schuberg Philis
// SPDX-License-Identifier: Apache-2.0
//! A multi-table database -- `directors`, `movies`, `actors`, and a
//! `movie_cast` junction table -- populated with Quentin Tarantino's and
//! the Coen Brothers' filmographies and their lead cast, then a `JOIN`
//! across all four tables.
//!
//! Every other example here is single-table; this one exercises
//! multi-table schema design and `JOIN` execution end to end, following
//! `crud.rs`'s pattern (copy `fixtures/empty.db` to a scratch path, build
//! the schema via an explicit `BEGIN`/`COMMIT` transaction).
//!
//! Run with: `cargo run --example movies`

use std::cell::RefCell;
use std::collections::HashMap;
use std::error::Error;
use std::path::Path;
use std::rc::Rc;

use sqlite_rs::btree::TableCursor;
use sqlite_rs::codegen::{compile_select_joined, compile_statement, resolve_from_table_schema};
use sqlite_rs::dump;
use sqlite_rs::format::format_query_value;
use sqlite_rs::parser::{parse_select, split_statements, ParseOutcome};
use sqlite_rs::schema::{read_schema, read_views};
use sqlite_rs::vdbe::{execute_transaction_step, execute_with_db};
use sqlite_rs::vfs::{PageSource, UnixVfs};

fn main() -> Result<(), Box<dyn Error>> {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/fixtures/empty.db");
    let scratch_dir =
        std::env::temp_dir().join(format!("sqlite-rs-movies-example-{}", std::process::id()));
    std::fs::create_dir_all(&scratch_dir)?;
    let scratch_db = scratch_dir.join("movies.db");
    std::fs::copy(&fixture, &scratch_db)?;

    let (header, pager) = dump::open(&UnixVfs, &scratch_db)?;
    let pager = Rc::new(RefCell::new(pager));

    let script = "
        CREATE TABLE directors(id INTEGER PRIMARY KEY, name TEXT);
        CREATE TABLE movies(id INTEGER PRIMARY KEY, title TEXT, year INTEGER, director_id INTEGER);
        CREATE TABLE actors(id INTEGER PRIMARY KEY, name TEXT);
        CREATE TABLE movie_cast(movie_id INTEGER, actor_id INTEGER, role TEXT, is_lead INTEGER);
        BEGIN;

        INSERT INTO directors(id, name) VALUES (1, 'Quentin Tarantino');
        INSERT INTO directors(id, name) VALUES (2, 'Coen Brothers');

        INSERT INTO movies(id, title, year, director_id) VALUES (1, 'Pulp Fiction', 1994, 1);
        INSERT INTO movies(id, title, year, director_id) VALUES (2, 'Kill Bill: Volume 1', 2003, 1);
        INSERT INTO movies(id, title, year, director_id) VALUES (3, 'Inglourious Basterds', 2009, 1);
        INSERT INTO movies(id, title, year, director_id) VALUES (4, 'Django Unchained', 2012, 1);
        INSERT INTO movies(id, title, year, director_id) VALUES (5, 'Once Upon a Time in Hollywood', 2019, 1);
        INSERT INTO movies(id, title, year, director_id) VALUES (6, 'Fargo', 1996, 2);
        INSERT INTO movies(id, title, year, director_id) VALUES (7, 'The Big Lebowski', 1998, 2);
        INSERT INTO movies(id, title, year, director_id) VALUES (8, 'No Country for Old Men', 2007, 2);
        INSERT INTO movies(id, title, year, director_id) VALUES (9, 'True Grit', 2010, 2);

        INSERT INTO actors(id, name) VALUES (1, 'John Travolta');
        INSERT INTO actors(id, name) VALUES (2, 'Uma Thurman');
        INSERT INTO actors(id, name) VALUES (3, 'Brad Pitt');
        INSERT INTO actors(id, name) VALUES (4, 'Christoph Waltz');
        INSERT INTO actors(id, name) VALUES (5, 'Jamie Foxx');
        INSERT INTO actors(id, name) VALUES (6, 'Leonardo DiCaprio');
        INSERT INTO actors(id, name) VALUES (7, 'Frances McDormand');
        INSERT INTO actors(id, name) VALUES (8, 'Jeff Bridges');
        INSERT INTO actors(id, name) VALUES (9, 'Javier Bardem');
        INSERT INTO actors(id, name) VALUES (10, 'Hailee Steinfeld');

        INSERT INTO movie_cast(movie_id, actor_id, role, is_lead) VALUES (1, 1, 'Vincent Vega', 1);
        INSERT INTO movie_cast(movie_id, actor_id, role, is_lead) VALUES (1, 2, 'Mia Wallace', 1);
        INSERT INTO movie_cast(movie_id, actor_id, role, is_lead) VALUES (2, 2, 'The Bride', 1);
        INSERT INTO movie_cast(movie_id, actor_id, role, is_lead) VALUES (3, 3, 'Lt. Aldo Raine', 1);
        INSERT INTO movie_cast(movie_id, actor_id, role, is_lead) VALUES (3, 4, 'Col. Hans Landa', 1);
        INSERT INTO movie_cast(movie_id, actor_id, role, is_lead) VALUES (4, 5, 'Django', 1);
        INSERT INTO movie_cast(movie_id, actor_id, role, is_lead) VALUES (4, 4, 'Dr. King Schultz', 1);
        INSERT INTO movie_cast(movie_id, actor_id, role, is_lead) VALUES (5, 6, 'Rick Dalton', 1);
        INSERT INTO movie_cast(movie_id, actor_id, role, is_lead) VALUES (5, 3, 'Cliff Booth', 1);
        INSERT INTO movie_cast(movie_id, actor_id, role, is_lead) VALUES (6, 7, 'Marge Gunderson', 1);
        INSERT INTO movie_cast(movie_id, actor_id, role, is_lead) VALUES (7, 8, 'The Dude', 1);
        INSERT INTO movie_cast(movie_id, actor_id, role, is_lead) VALUES (8, 9, 'Anton Chigurh', 1);
        INSERT INTO movie_cast(movie_id, actor_id, role, is_lead) VALUES (9, 10, 'Mattie Ross', 1);

        COMMIT;
    ";

    let mut autocommit = true;
    for stmt in split_statements(script) {
        let (schemas, views) = {
            let borrowed = pager.borrow();
            let mut schema_cursor = TableCursor::new(&*borrowed, &header, 1);
            let schemas = read_schema(&mut schema_cursor, header.text_encoding)?;
            let mut view_cursor = TableCursor::new(&*borrowed, &header, 1);
            let views = read_views(&mut view_cursor, header.text_encoding)?;
            (schemas, views)
        };

        let program = compile_statement(&stmt, &schemas, &views).map_err(|e| e.to_string())?;
        let (_, ac) = execute_transaction_step(&program, Rc::clone(&pager), header, autocommit)
            .map_err(|e| e.to_string())?;
        autocommit = ac;
    }

    // Every movie, its director, and its lead cast -- a 4-table JOIN.
    let schemas = {
        let borrowed = pager.borrow();
        let mut schema_cursor = TableCursor::new(&*borrowed, &header, 1);
        read_schema(&mut schema_cursor, header.text_encoding)?
    };
    let query = "
        SELECT movies.year, movies.title, directors.name, actors.name, movie_cast.role
        FROM movies
        JOIN directors ON movies.director_id = directors.id
        JOIN movie_cast ON movie_cast.movie_id = movies.id
        JOIN actors ON actors.id = movie_cast.actor_id
        WHERE movie_cast.is_lead = 1
        ORDER BY movies.year
    ";
    let select = match parse_select(query) {
        ParseOutcome::Accepted(select) => *select,
        _ => return Err("failed to parse the JOIN query".into()),
    };
    let from = select.from.as_ref().ok_or("SELECT has no FROM clause")?;
    let mut joined_schemas =
        vec![resolve_from_table_schema(&from.first, &schemas).map_err(|e| e.to_string())?];
    for join in &from.joins {
        joined_schemas
            .push(resolve_from_table_schema(&join.table, &schemas).map_err(|e| e.to_string())?);
    }
    let program = compile_select_joined(&select, &joined_schemas, &schemas, &HashMap::new())
        .map_err(|e| e.to_string())?;
    let source: Rc<dyn PageSource> = pager;
    let rows = execute_with_db(&program, source, header).map_err(|e| e.to_string())?;

    println!("Tarantino & Coen Brothers, by year:");
    for row in rows {
        let rendered: Vec<String> = row
            .iter()
            .map(|v| String::from_utf8_lossy(&format_query_value(v)).into_owned())
            .collect();
        println!("  {}", rendered.join(" | "));
    }

    std::fs::remove_dir_all(&scratch_dir).ok();
    Ok(())
}
