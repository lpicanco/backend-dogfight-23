use std::time::Duration;

use actix_web::{get, post, web, App, HttpResponse, HttpServer, Responder};
use chrono::NaiveDate;
use serde::Deserialize;
use serde_derive::Serialize;
use sqlx::postgres::PgPoolOptions;
use sqlx::{PgPool, Row};
use uuid::Uuid;
use validator::{Validate, ValidationError};

#[derive(Debug, Validate, Deserialize, sqlx::FromRow, Serialize)]
struct Pessoa {
    id: Option<Uuid>,

    #[validate(required, length(min = 1, max = 32))]
    apelido: Option<String>,

    #[validate(required, length(min = 1, max = 100))]
    nome: Option<String>,

    #[validate(required)]
    nascimento: Option<NaiveDate>,

    #[serde(default)]
    #[validate(custom = "validate_stack")]
    stack: Option<Vec<String>>,
}

fn validate_stack(stack: &[String]) -> Result<(), ValidationError> {
    for item in stack.iter() {
        if item.len() > 32 {
            return Err(ValidationError::new("stack_too_long"));
        }
    }
    Ok(())
}

#[post("/pessoas")]
async fn create_pessoa(pool: web::Data<PgPool>, pessoa: web::Json<Pessoa>) -> impl Responder {
    if let Err(errors) = pessoa.validate() {
        return HttpResponse::UnprocessableEntity().json(errors);
    }

    let id = Uuid::new_v4();
    let stack_str = match &pessoa.stack {
        Some(s) => s.join(" "),
        None => String::from(""),
    };
    let search_text = format!(
        "{} {} {}",
        pessoa.nome.clone().unwrap(),
        pessoa.apelido.clone().unwrap(),
        stack_str
    )
    .to_lowercase();
    let result = sqlx::query(
        "INSERT INTO pessoas (id, apelido, nome, nascimento, stack, search_vector) VALUES \
        ($1, $2, $3, $4, $5, $6)",
    )
    .bind(id)
    .bind(&pessoa.apelido)
    .bind(&pessoa.nome)
    .bind(&pessoa.nascimento)
    .bind(&pessoa.stack)
    .bind(&search_text)
    .execute(pool.get_ref())
    .await;

    match result {
        Ok(_) => HttpResponse::Created()
            .append_header(("Location", format!("/pessoas/{}", id)))
            .finish(),
        Err(e) => {
            println!("Failed to execute query: {}", e);
            HttpResponse::InternalServerError().finish()
        }
    }
}

#[get("/pessoas/{id}")]
async fn get_pessoa_by_id(
    pool: web::Data<PgPool>,
    id: web::Path<Uuid>,
) -> actix_web::Result<impl Responder> {
    let result = sqlx::query_as::<_, Pessoa>("SELECT * FROM pessoas WHERE id = $1")
        .bind(&id.into_inner())
        .fetch_optional(pool.get_ref())
        .await;

    if let Err(e) = result {
        println!("Failed to execute query: {}", e);
        return Ok(HttpResponse::InternalServerError().finish());
    }

    match result.unwrap() {
        Some(pessoa) => Ok(HttpResponse::Ok().json(pessoa)),
        None => Ok(HttpResponse::NotFound().finish()),
    }
}

#[derive(Deserialize)]
struct SearchQuery {
    t: String,
}

#[get("/pessoas")]
async fn search_pessoa(
    pool: web::Data<PgPool>,
    query: web::Query<SearchQuery>,
) -> actix_web::Result<impl Responder> {
    let result =
        sqlx::query_as::<_, Pessoa>("SELECT * FROM pessoas WHERE search_vector ~ $1 LIMIT 50")
            .bind(&query.t.to_lowercase())
            .fetch_all(pool.get_ref())
            .await;

    match result {
        Ok(pessoas) => Ok(HttpResponse::Ok().json(pessoas)),
        Err(e) => {
            println!("Failed to execute query: {}", e);
            Ok(HttpResponse::InternalServerError().finish())
        }
    }
}

#[get("/contagem-pessoas")]
async fn count_pessoas(pool: web::Data<PgPool>) -> actix_web::Result<impl Responder> {
    let result = sqlx::query("SELECT COUNT(id) FROM pessoas")
        .fetch_one(&**pool)
        .await;

    match result {
        Ok(count) => Ok(HttpResponse::Ok().body(count.get::<i64, usize>(0).to_string())),
        Err(e) => {
            println!("Failed to execute query: {}", e);
            Ok(HttpResponse::InternalServerError().finish())
        }
    }
}

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    let database_url = "postgres://dogfight_user:dogfight_pass@db/dogfight";
    let pool = PgPoolOptions::new()
        .max_connections(150)
        .connect(&database_url)
        .await
        .expect("🔥 Failed to create DB pool");

    HttpServer::new(move || {
        App::new()
            .app_data(web::Data::new(pool.clone()))
            .service(create_pessoa)
            .service(get_pessoa_by_id)
            .service(search_pessoa)
            .service(count_pessoas)
    })
    .client_request_timeout(Duration::from_secs(30))
    .bind("0.0.0.0:9999")?
    .run()
    .await
}
