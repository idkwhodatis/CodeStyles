// C++ templates, containers and control flow
#include <string>
#include <vector>
struct Item{std::string name;int count;};
template<class Items> int total(const Items& items){int sum=0;for(const auto& item:items){if(item.count>0){sum+=item.count;}}return sum;}
