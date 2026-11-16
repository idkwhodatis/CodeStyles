"""Make TinyGo reactor init an eager start function, without changing code.

Equivalent ABI intent to dprint-plugin-shfmt's Binaryen setStart/removeExport.
This bounded binary rewrite preserves all other sections byte-for-byte.
"""
import pathlib,sys

def read(data, p):
    result=0;shift=0
    while True:
        b=data[p];p+=1;result|=(b&127)<<shift
        if not b&128:return result,p
        shift+=7
        if shift>35:raise ValueError('invalid u32 LEB')

def leb(value):
    result=bytearray()
    while True:
        b=value&127;value>>=7;result.append(b|(128 if value else 0))
        if not value:return bytes(result)

def section(kind,payload):return bytes([kind])+leb(len(payload))+payload

def patch(data):
    assert data[:8]==b'\0asm\x01\0\0\0'
    sections=[];p=8;init=None;seen=set()
    while p<len(data):
        kind=data[p];size,p=read(data,p+1);payload=data[p:p+size];p+=size
        assert len(payload)==size
        if kind:assert kind not in seen;seen.add(kind)
        assert kind!=8,'unexpected existing start section'
        if kind==7:
            count,q=read(payload,0);exports=[]
            for _ in range(count):
                start=q;n,q=read(payload,q);name=payload[q:q+n];q+=n;export_kind=payload[q];index,q=read(payload,q+1)
                if name==b'_initialize':
                    assert export_kind==0 and init is None;init=index
                else:exports.append(payload[start:q])
            assert q==len(payload)
            payload=leb(len(exports))+b''.join(exports)
        sections.append((kind,payload))
    assert init is not None,'missing _initialize export'
    output=bytearray(data[:8]);inserted=False
    for kind,payload in sections:
        if kind>8 and not inserted:output+=section(8,leb(init));inserted=True
        output+=section(kind,payload)
    if not inserted:output+=section(8,leb(init))
    return bytes(output)

if __name__=='__main__':pathlib.Path(sys.argv[2]).write_bytes(patch(pathlib.Path(sys.argv[1]).read_bytes()))
