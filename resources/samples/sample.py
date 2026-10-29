# Tune Python formatting without running this code.
from dataclasses import dataclass
from pathlib import Path

@dataclass
class Preview:
    name:str
    enabled:bool=True

    def greeting(self,prefix:str='Hello')->str:
        return f'{prefix}, {self.name}!'

def summarize(items:list[Preview],limit:int=3)->dict[str,object]:
    names=[item.name for item in items if item.enabled]
    options={'enabled':True,'count':len(names),'labels':names[:limit]}
    return options

async def read_example(path:Path)->str:
    with path.open(encoding='utf-8') as source:
        return source.read()

example=Preview('CodeStyles')
result=summarize([example,Preview('Python',False)])
