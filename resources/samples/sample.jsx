import {useState} from 'react';

/** A component with expressions, fragments and nested elements. */
const entries=[{id:1,label:'Source',enabled:true},{id:2,label:'Preview',enabled:false}];
export function Preview({title='CodeStyles',...props}) {
const [selected,setSelected]=useState(1);
return <main {...props} className='preview' aria-label={title}>
{/* Choose an item to compare its formatting. */}
<header><h1>{title}</h1><p>Compare <strong>saved</strong> &amp; draft styles.</p></header>
<ul>{entries.map(({id,label,enabled})=><li key={id} data-enabled={enabled}>
<button type='button' disabled={!enabled} onClick={()=>setSelected(id)} style={{display:'flex',gap:8}}>{label}</button>
</li>)}</ul>
{selected>0?<><hr/><span>Selected: {selected}</span></>:<span>No selection</span>}
<input placeholder='A longer placeholder for attribute wrapping' readOnly value={title}/>
</main>
}
