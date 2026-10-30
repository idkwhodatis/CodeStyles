package main
import "fmt"
// A compact Go formatting preview.
type Item struct{Name string;Count int}
func main(){items:=[]Item{{Name:"Go",Count:3}};for _,item:=range items{fmt.Printf("%s: %d\n",item.Name,item.Count)}}
