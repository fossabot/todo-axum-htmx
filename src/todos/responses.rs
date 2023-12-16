use axum::{
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
};
use axum_extra::extract::Form;

use serde::Deserialize;
use sqlx::PgPool;

use crate::utils;
use crate::{todos::Todo, utils::HtmlTemplate};

use super::templates;

#[derive(Deserialize)]
pub struct TodoCreateParams {
    description: String,
}

async fn get_todos(pool: &PgPool) -> Result<Vec<Todo>, (StatusCode, String)> {
    sqlx::query_as!(
        Todo,
        "select id, done, description, position from todos ORDER BY position desc"
    )
    .fetch_all(pool)
    .await
    .map_err(utils::internal_error)
}

async fn delete_todos(todos: Vec<Todo>, pool: &PgPool) -> Result<(), (StatusCode, String)> {
    let delete_ids = todos.iter().map(|t| t.id as i32).collect::<Vec<_>>();
    // https://github.com/launchbadge/sqlx/blob/main/FAQ.md#how-can-i-do-a-select--where-foo-in--query
    sqlx::query!("delete from todos where id = ANY($1)", &delete_ids)
        .execute(pool)
        .await
        .map_err(utils::internal_error)?;
    Ok(())
}

async fn render_all_todos(
    pool: &PgPool,
) -> Result<templates::TodosInnerTemplate, (StatusCode, String)> {
    let todos = get_todos(pool).await?;
    Ok(render_todos(todos))
}

fn render_todos(todos: Vec<Todo>) -> templates::TodosInnerTemplate {
    let todos: Vec<templates::TodoLiTemplate> =
        todos.into_iter().map(|t| t.into()).collect::<Vec<_>>();
    templates::TodosInnerTemplate {
        todos, // todos: templates::TodosInnerTemplate { todos },
    }
}

pub async fn create(
    State(pool): State<PgPool>,
    Form(params): Form<TodoCreateParams>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    sqlx::query!(
        "INSERT INTO todos (description,position) VALUES ($1,((select max(position) from todos) + 1));",
        params.description,
    )
    .execute(&pool)
    .await
    .map_err(utils::internal_error)?;

    let template = render_all_todos(&pool).await?;

    let mut headers = HeaderMap::new();
    headers.insert(
        "HX-Trigger",
        "todoFormReset"
            .parse()
            .expect("should be able to create a HX-Trigger header"),
    );
    Ok((headers, HtmlTemplate(template)))
}

pub async fn list(State(pool): State<PgPool>) -> Result<impl IntoResponse, (StatusCode, String)> {
    let inner_template = render_all_todos(&pool).await?;
    let template = templates::TodosUlTemplate {
        todos: inner_template,
    };
    Ok(HtmlTemplate(template))
}

pub async fn move_complete_to_bottom(
    State(pool): State<PgPool>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let mut todos = get_todos(&pool).await?;
    todos.sort_by(|a, b| a.position.cmp(&b.position));
    let (mut completed, mut pending): (Vec<_>, Vec<_>) = todos.into_iter().partition(|t| t.done);
    completed.append(&mut pending);
    let positions = completed
        .iter()
        .enumerate()
        .map(|(position, todo)| (position as i32, todo.id as i32))
        .collect::<Vec<_>>();
    set_positions(positions, &pool).await?;
    let template = render_all_todos(&pool).await?;
    Ok(HtmlTemplate(template))
}

pub async fn delete_completed(
    State(pool): State<PgPool>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let todos = get_todos(&pool).await?;
    let (completed, pending): (Vec<_>, Vec<_>) = todos.into_iter().partition(|t| t.done);

    // Delete the completed ones
    delete_todos(completed, &pool).await?;

    let template = render_todos(pending);
    Ok(HtmlTemplate(template))
}

#[derive(Deserialize)]
pub struct TodoOrderingParams {
    order: Vec<String>,
}

// Given a vec of (position, id), set the position for each todo by id
async fn set_positions(
    position_data: Vec<(i32, i32)>,
    pool: &PgPool,
) -> Result<(), (StatusCode, String)> {
    let positions = position_data
        .clone()
        .into_iter()
        .map(|(pos, _)| pos)
        .collect::<Vec<_>>();
    let ids = position_data
        .into_iter()
        .map(|(_, id)| id)
        .collect::<Vec<_>>();
    sqlx::query!(
        "update todos as original
         set position=new.position
         from (select unnest($1::int4[]) as position, unnest($2::int4[]) as id) as new
         where original.id=new.id;",
        &positions[..],
        &ids[..],
    )
    .execute(pool)
    .await
    .map_err(utils::internal_error)?;
    Ok(())
}

pub async fn update_order(
    State(pool): State<PgPool>,
    Form(params): Form<TodoOrderingParams>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    println!("order params: {:?}", params.order);
    let positions: Vec<(i32, i32)> = params
        .order
        .iter()
        .rev()
        .enumerate()
        .map(|(pos, id)| (pos as i32, id.parse().unwrap_or(0)))
        .collect::<Vec<_>>();
    set_positions(positions, &pool).await?;

    let template = render_all_todos(&pool).await?;
    Ok(HtmlTemplate(template))
}

#[derive(Debug, Deserialize)]
pub struct TodoUpdateParams {
    done: Option<String>,
}

#[derive(Debug, Deserialize)]
enum CheckBox {
    On,
    Off,
}

impl From<CheckBox> for bool {
    fn from(val: CheckBox) -> Self {
        match val {
            CheckBox::On => true,
            CheckBox::Off => false,
        }
    }
}

impl From<String> for CheckBox {
    fn from(val: String) -> Self {
        if val == "on" {
            CheckBox::On
        } else {
            CheckBox::Off
        }
    }
}

pub async fn update(
    Path(todo_id): Path<i32>,
    State(pool): State<PgPool>,
    Form(params): Form<TodoUpdateParams>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let check_box: CheckBox = params.done.unwrap_or(String::from("Off")).into();
    let check_box: bool = check_box.into();

    sqlx::query!(
        "UPDATE todos set done = $1 where id = $2",
        check_box,
        todo_id,
    )
    .execute(&pool)
    .await
    .map_err(utils::internal_error)?;

    let template = render_all_todos(&pool).await?;
    Ok(HtmlTemplate(template))
}

pub async fn delete(
    Path(todo_id): Path<i32>,
    State(pool): State<PgPool>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    sqlx::query!("DELETE FROM todos where id = $1", todo_id)
        .execute(&pool)
        .await
        .map_err(utils::internal_error)?;

    let template = render_all_todos(&pool).await?;
    Ok(HtmlTemplate(template))
}
