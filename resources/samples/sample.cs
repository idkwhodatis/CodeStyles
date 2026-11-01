// C# source only: no project or solution evaluation
using System;
using System.Collections.Generic;
public record Item(string Name,int Count);
public class Example{public static int Total(IEnumerable<Item> items){var sum=0;foreach(var item in items){if(item.Count>0){sum+=item.Count;}}return sum;}}
