-- Tables, functions and control flow; never executed.
local options={name="CodeStyles",enabled=true,values={1,2,3}}
local function total(values)
local result=0
for _,value in ipairs(values) do result=result+value end
return result
end
if options.enabled then print(options.name,total(options.values)) end
return options
