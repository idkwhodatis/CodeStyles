// Rust syntax tour: formatted, never compiled or executed.
use std::collections::HashMap;

#[derive(Clone,Debug)]
struct Preview<'a>{name:&'a str,enabled:bool}

impl<'a> Preview<'a>{
    fn greeting(&self)->String{format!("Hello, {}!",self.name)}
}

fn summarize(items:&[Preview<'_>])->HashMap<String,usize>{
    let mut counts=HashMap::new();
    for item in items.iter().filter(|item|item.enabled){
        *counts.entry(item.greeting()).or_insert(0)+=1;
    }
    counts
}

async fn example()->Result<(),String>{
    let items=vec![Preview{name:"CodeStyles",enabled:true}];
    match summarize(&items).len(){0=>Err("No previews".into()),_=>Ok(())}
}
