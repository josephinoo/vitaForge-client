fn main() {
    dotenvy::dotenv().ok();
    let v = vitaforge_client::api::version().unwrap();
    let apps = vitaforge_client::api::snapshot().unwrap();
    println!("{v:?} parsed={}", apps.len());
    println!("{:?}", apps.iter().find(|a| a.data_url.is_some()).map(|a| (&a.name, &a.data_url)));
    println!("vita: {:?}", vitaforge_client::vita::find().map(|v| (v.root, v.apps.len())));
}
