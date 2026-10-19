import type {User} from './types';
import {readFile,writeFile as save} from 'node:fs/promises';

/** A compact tour of TypeScript syntax. */
export interface Preview<T> {readonly value:T; label?:string; render(value:T):string}
type State='saved'|'draft';
type Pair=[name:string,count:number];
type Flags<T>={readonly [K in keyof T]?:boolean};
type Result<T>=T extends Promise<infer U>?U:T;
export enum Mode {Compact='compact',Expanded='expanded'}
const settings={width:80,tabs:false,tags:['source','preview']} satisfies Record<string,unknown>;
const points:readonly Pair[]=[['alpha',1],['beta',2]];

export class Store<T extends {id:number}> {
private items:T[]=[];
constructor(public readonly name:string){}
add(...items:T[]):this {this.items.push(...items);return this}
get first():T|undefined {return this.items[0]}
}
export function describe<T extends User>(user:T,options:Partial<Preview<T>>={}):string {
const {label='Guest',...rest}=options; // Preserve the chosen label.
const name=user?.name??label;
if(name.length>0&&Object.keys(rest).length===0){return `Hello, ${name}!`}else{return 'Welcome'}
}
export async function load(path:string):Promise<unknown> {
try {const text=await readFile(path,'utf8');return JSON.parse(text) as unknown}
catch(error:unknown){throw new Error(String(error))}
finally {console.debug('Loaded')}
}
for(const [name,count] of points){console.log(name,count>1?'many':'one')}
export {save};
