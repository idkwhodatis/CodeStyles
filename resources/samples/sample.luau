-- Luau types and functions; never executed.
type Options={name:string,values:{number}}
local options:Options={name="CodeStyles",values={1,2,3}}
local function total(values:{number}):number
local result=0
for _,value in ipairs(values) do result+=value end
return result
end
print(options.name,total(options.values))
return options
