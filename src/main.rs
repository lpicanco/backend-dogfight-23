use std::time::Duration;

use actix_web::{get, post, web, App, HttpResponse, HttpServer, Responder};
use chrono::NaiveDate;
use deadpool_redis::{Config, Runtime};
use redis::AsyncCommands;
use serde::Deserialize;
use serde_derive::Serialize;
use serde_json::to_string;
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
async fn create_pessoa(
    pool: web::Data<PgPool>,
    redis_pool: web::Data<deadpool_redis::Pool>,
    mut pessoa: web::Json<Pessoa>,
) -> impl Responder {
    if let Err(errors) = pessoa.validate() {
        return HttpResponse::UnprocessableEntity().json(errors);
    }

    let id = Uuid::new_v4();
    pessoa.id = Some(id);
    let serialized_person = to_string(&pessoa).unwrap();
    let mut redis = redis_pool.get_ref().get().await.unwrap();
    let exists = redis
        .sadd::<_, _, i32>("apelidos", &pessoa.apelido)
        .await
        .unwrap();
    if exists == 0 {
        return HttpResponse::UnprocessableEntity().finish();
    }

    redis
        .set::<_, _, ()>(id.to_string(), &serialized_person)
        .await
        .unwrap();
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
    redis_pool: web::Data<deadpool_redis::Pool>,
    id: web::Path<Uuid>,
) -> actix_web::Result<impl Responder> {
    let mut redis = redis_pool.get_ref().get().await.unwrap();
    let pessoa_json: Option<String> = redis.get(id.clone().to_string()).await.unwrap_or(None);
    if let Some(json_data) = pessoa_json {
        return Ok(HttpResponse::Ok()
            .content_type("application/json")
            .body(json_data));
    }

    let result = sqlx::query_as::<_, Pessoa>(
        "SELECT id, apelido, nome, nascimento, stack FROM pessoas WHERE id = $1",
    )
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
    let result = sqlx::query_as::<_, Pessoa>(
        "\
        SELECT id, apelido, nome, nascimento, stack FROM pessoas WHERE search_vector ~ $1 LIMIT 50",
    )
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
        .max_connections(50)
        .acquire_timeout(Duration::from_secs(120))
        .test_before_acquire(false)
        .connect(&database_url)
        .await
        .expect("🔥 Failed to create DB pool");

    let redis_client = Config::from_url("redis://redis:6379/");
    let redis_pool = redis_client.create_pool(Some(Runtime::Tokio1)).unwrap();

    HttpServer::new(move || {
        App::new()
            .app_data(web::Data::new(pool.clone()))
            .app_data(web::Data::new(redis_pool.clone()))
            .service(create_pessoa)
            .service(get_pessoa_by_id)
            .service(search_pessoa)
            .service(count_pessoas)
    })
    .bind("0.0.0.0:9999")?
    .run()
    .await
}
