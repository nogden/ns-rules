#[derive(Debug)]
enum Form<'s> {
    Collection(Collection),
    Symbol(&'s str),
    Keyword(&'s str),
    Other(&'s str),
}

#[derive(Debug)]
enum CollectionType {
    List, Vector, Map, // Set not needed
}

#[derive(Debug)]
struct Collection {
    collection_type: CollectionType,
    start: usize,
    elements: Vec<Form>,
}

fn read(code: &str) -> Vec<Form> {
    let stack = Vec::new();
    let mut chars = code.chars_index();

    while let Some((i, c)) = chars.next() {
        match c {
            '(' => stack.push(Form::Collection(Collection {
                collection_type: CollectionType::List,
                start: i,

            })),
            '[' => stack.push(Vector(i)),
            '{' => stack.push(Map(i)),
            '0'..'9' =>
        }
    }
}
