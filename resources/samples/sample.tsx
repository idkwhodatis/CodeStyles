import {useState,type ReactNode} from 'react';

interface Entry {readonly id:number;label:string;enabled?:boolean}
type Props={title?:string;children?:ReactNode;onSelect?:(entry:Entry)=>void};
const entries:Entry[]=[{id:1,label:'Source',enabled:true},{id:2,label:'Preview',enabled:false}];
/** Typed props and generic calls alongside JSX syntax. */
export function Preview({title='CodeStyles',children,onSelect}:Props) {
const [selected,setSelected]=useState<number|null>(null);
const choose=(entry:Entry):void=>{setSelected(entry.id);onSelect?.(entry)};
return <main className='preview' aria-label={title}>
{/* Keep comments with the section they describe. */}
<header><h1>{title}</h1><p>Compare <strong>saved</strong> &amp; draft styles.</p></header>
<ul>{entries.map(entry=><li key={entry.id} data-enabled={entry.enabled}>
<button type='button' disabled={!entry.enabled} onClick={()=>choose(entry)} style={{display:'flex',gap:8}}>{entry.label}</button>
</li>)}</ul>
{selected!==null?<><hr/><span>Selected: {selected}</span></>:<span>No selection</span>}
<input placeholder='A longer placeholder for attribute wrapping' readOnly value={title}/>
{children??<footer>Ready to preview</footer>}
</main>
}
