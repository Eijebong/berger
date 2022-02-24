use anyhow::Result;
use once_cell::sync::OnceCell;
use sqlx::PgPool;

pub static POOL: OnceCell<PgPool> = OnceCell::new();

pub async fn init_pool(url: &str) -> Result<()> {
    let pool = PgPool::connect(url).await?;
    POOL.set(pool).unwrap();

    Ok(())
}
