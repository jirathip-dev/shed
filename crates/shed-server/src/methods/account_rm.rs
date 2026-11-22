use crate::call::{Call, Reply, blocking, empty};
use crate::proto::{AccountName, ErrorCode, RpcError};

pub(super) async fn call(cx: &mut Call<'_>) -> Reply {
    let AccountName { provider, name } = cx.params()?;
    let (store, record) = crate::accounts::open(cx, &provider).await?;
    if let Some(login) = crate::accounts::login_running(&*cx.reg()?, &provider, &name) {
        return Err(RpcError::new(ErrorCode::StillRunning, format!("its login {login} runs")));
    }
    store.folder(&record, &name)?;
    if name != shed_accounts::store::DEFAULT {
        shed_accounts::check::logout(&store, &record, &name).await?;
    }
    blocking(move || store.remove(&record, &name)).await??;
    Ok(empty())
}
