use crate::call::{Call, Reply, blocking, json};
use crate::proto::{AccountMoves, AccountUse};

pub(super) async fn call(cx: &mut Call<'_>) -> Reply {
    let AccountUse { provider, name } = cx.params()?;
    let (store, record) = crate::accounts::open(cx, &provider).await?;
    let Some(name) = name else {
        blocking(move || store.prefer(&record, None)).await??;
        return json(&AccountMoves::default());
    };
    let account = store.account(&record, &name)?;
    shed_accounts::choice::usable(&account)?;
    store.prefer(&record, Some(&name))?;
    let inputs = cx.reg()?.use_account(&record, &name);
    let mut moves = AccountMoves::default();
    for (terminal, input) in inputs {
        match input {
            Ok(input) => {
                input.write().await?;
                moves.moved.push(terminal);
            }
            Err(why) => moves.stayed.push(crate::proto::Stayed { terminal, why }),
        }
    }
    json(&moves)
}
