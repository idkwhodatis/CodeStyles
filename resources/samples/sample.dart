// Dart source formatting also applies to Flutter widget source.
class Greeting{final String name;const Greeting(this.name);String message()=> 'Hello, $name!';}
void main(){const names=['Dart','Flutter'];for(final name in names){print(Greeting(name).message());}}
