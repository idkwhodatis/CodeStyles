import {readFile,writeFile as save} from 'node:fs/promises';

/** Build a preview from ordinary JavaScript values. */
const options={name:'CodeStyles',enabled:true,tags:['saved','draft'],nested:{width:80}};
const {name,...rest}=options;
const values=[1,2,3,...[4,5]].filter(value=>value%2===1).map(value=>({value,label:`Item ${value}`}));
const pattern=/preview\s+/gi;
export class Counter {
#value=0;
constructor(start=0){this.#value=start}
get value(){return this.#value}
increment(step=1){this.#value+=step;return this}
}
export function describe({label='Preview',items=[]}={}) {
// Keep the fallback next to the optional property access.
const first=items[0]?.label??label;
if(items.length>0&&first){return `${first}: ${items.length}`}else{return 'Empty'}
}
export async function load(path) {
try {const text=await readFile(path,'utf8');return JSON.parse(text)}
catch(error){throw new Error(String(error))}
finally {console.debug('Loaded')}
}
function* counts(limit){for(let index=0;index<limit;index++){yield index}}
for(const value of counts(3)){switch(value){case 0:console.log('start');break;default:console.log(value)}}
export {name,rest,values,pattern,save};
