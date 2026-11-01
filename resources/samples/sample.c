/* C functions, structs, arrays and control flow */
#include <stddef.h>
struct item{const char *name;int count;};
int total(const struct item *items,size_t length){int sum=0;for(size_t i=0;i<length;++i){if(items[i].count>0){sum+=items[i].count;}}return sum;}
