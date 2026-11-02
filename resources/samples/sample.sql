-- A portable query without templates or procedural statements.
select category,count(*) as item_count,sum(price) as total from items where active = 1 and price > 10 group by category having count(*) > 1 order by total desc;
