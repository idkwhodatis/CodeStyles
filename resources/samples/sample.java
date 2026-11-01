// Java records, collections and control flow
import java.util.List;
class Example{record Item(String name,int count){} static int total(List<Item> items){int sum=0;for(var item:items){if(item.count()>0){sum+=item.count();}}return sum;}}
