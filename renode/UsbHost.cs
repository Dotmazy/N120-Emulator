using System;
using System.IO;
using System.Net;
using System.Net.Sockets;
using System.Collections.Generic;
using System.Collections.Concurrent;
using System.Threading;
using Antmicro.Renode.Core.USB;
namespace Antmicro.Renode.Peripherals.N120 {
    // Serialize SETUP and status stages. Wait for the device to arm EP0 OUT;
    // never sleep for a guessed firmware recovery interval or block its CPU thread.
    public sealed class ReadyConnection:IUSBConnection {
        internal readonly object IoSync;private readonly IUSBConnection inner;private readonly Func<bool> ready;private readonly bool endSessionOnDispose;
        private readonly WaitHandle changed;private readonly CancellationTokenSource stop=new CancellationTokenSource();
        private readonly BlockingCollection<Tuple<Action,bool>> operations=new BlockingCollection<Tuple<Action,bool>>(128);
        public Func<bool> DataReady=()=>true;
        // Firmware can arm/fill the next IN packet inside the previous XFRC
        // handler. Do not acknowledge it until that handler has cleared XFRC;
        // otherwise its W1C consumes both completions and STATUS OUT stays NAKed.
        public Func<bool> ReadReady=()=>true;
        public Func<bool> Stalled=()=>false;
        public Action BeforeSetup=()=>{};
        private readonly Thread worker;private int disposed;public int Scheduled,Executed,Waits;
        public ReadyConnection(IUSBConnection connection,Func<bool> predicate,WaitHandle signal,object ioSync,bool endSessionOnDispose=true){IoSync=ioSync;inner=connection;ready=predicate;this.endSessionOnDispose=endSessionOnDispose;changed=signal;worker=new Thread(Run){IsBackground=true,Name="N120 USB host"};worker.Start();}
        private void Run(){try{foreach(var operation in operations.GetConsumingEnumerable(stop.Token)){while(true){stop.Token.ThrowIfCancellationRequested();lock(IoSync){if(ready()&&(!operation.Item2||DataReady())){operation.Item1();Executed++;break;}Waits++;}if(WaitHandle.WaitAny(new[]{changed,stop.Token.WaitHandle})==1)return;}}}catch(OperationCanceledException){}catch(ObjectDisposedException){}}
        public void Schedule(Action action,bool data=false){Scheduled++;operations.Add(Tuple.Create(action,data),stop.Token);}
        public void Flush(CancellationToken cancel){var done=new System.Threading.Tasks.TaskCompletionSource<bool>();Schedule(()=>done.TrySetResult(true));done.Task.Wait(cancel);}
        public IUSBPipeSetup ConnectEndpointSetup(byte ep){return new ReadySetup(inner.ConnectEndpointSetup(ep),this);}
        public IUSBPipeRead ConnectEndpointRead(byte ep){return inner.ConnectEndpointRead(ep);}
        public IUSBPipeWrite ConnectEndpointWrite(byte ep){return inner.ConnectEndpointWrite(ep);}
        public void Dispose(){if(Interlocked.Exchange(ref disposed,1)!=0)return;stop.Cancel();operations.CompleteAdding();if(Thread.CurrentThread!=worker&&worker.Join(1000)){operations.Dispose();stop.Dispose();}if(endSessionOnDispose)inner.Dispose();}
    }
    public sealed class ReadySetup:IUSBPipeSetup {
        private readonly IUSBPipeSetup pipe;private readonly ReadyConnection owner;
        public ReadySetup(IUSBPipeSetup p,ReadyConnection connection){pipe=p;owner=connection;}
        public event Action NewPacket {add{pipe.NewPacket+=value;}remove{pipe.NewPacket-=value;}}
        public bool TryRead(out byte[] data){lock(owner.IoSync){if(!owner.ReadReady()){data=null;return false;}return pipe.TryRead(out data);}}
        public void Write(byte[] data){var bytes=(byte[])data.Clone();owner.Schedule(()=>{if(!owner.Stalled())pipe.Write(bytes);},true);}
        public void SetupPacketWrite(SetupPacket packet){owner.Schedule(()=>{owner.BeforeSetup();pipe.SetupPacketWrite(packet);});}
    }
    // USB/IP 1.1.1 control transport. Firmware DFU uses EP0 only. Other
    // endpoint types are rejected, never acknowledged with synthetic data.
    public sealed class LocalUsbIp:IDisposable {
        private readonly TcpListener listener;
        private readonly CancellationTokenSource stop=new CancellationTokenSource();
        private readonly List<TcpClient> clients=new List<TcpClient>();
        private readonly List<Thread> workers=new List<Thread>();
        private readonly Thread accept;
        private readonly Func<byte[],byte[],CancellationToken,byte[]> transfer;
        private readonly Action disconnect;
        private readonly Action<string> log;
        private readonly object ownership=new object();private TcpClient owner;private bool imported;
        public int Port {get;private set;}
        public int ClientCount {get{lock(clients)return clients.Count;}}
        public bool Imported {get{lock(ownership)return imported;}}
        public LocalUsbIp(int port,Func<byte[],byte[],CancellationToken,byte[]> transact,Action release,Action<string> diagnostics){
            transfer=transact;disconnect=release;log=diagnostics;
            listener=new TcpListener(IPAddress.Loopback,port);listener.Start();Port=((IPEndPoint)listener.LocalEndpoint).Port;
            accept=new Thread(Accept){IsBackground=true,Name="N120 USB/IP accept"};accept.Start();
        }
        private void Accept(){try{while(!stop.IsCancellationRequested){
            var client=listener.AcceptTcpClient();client.NoDelay=true;client.ReceiveTimeout=5000;
            lock(clients){if(clients.Count>=8){client.Close();continue;}clients.Add(client);}
            var worker=new Thread(()=>Serve(client)){IsBackground=true,Name="N120 USB/IP client"};lock(workers)workers.Add(worker);worker.Start();
        }}catch(SocketException e){if(!stop.IsCancellationRequested)log(e.Message);}catch(ObjectDisposedException){}}
        private static byte[] Read(NetworkStream stream,int length){if(length<0||length>1048576)throw new IOException("USB/IP transfer exceeds 1 MiB");var result=new byte[length];int n=0;while(n<length){int got=stream.Read(result,n,length-n);if(got==0)throw new EndOfStreamException();n+=got;}return result;}
        private static uint U32(byte[] bytes,int offset){return (uint)(bytes[offset]<<24|bytes[offset+1]<<16|bytes[offset+2]<<8|bytes[offset+3]);}
        private static void Put(byte[] bytes,int offset,uint value){for(int i=0;i<4;i++)bytes[offset+i]=(byte)(value>>(24-8*i));}
        private byte[] Control(byte[] setup,byte[] data){using(var timeout=CancellationTokenSource.CreateLinkedTokenSource(stop.Token)){timeout.CancelAfter(5000);return transfer(setup,data,timeout.Token);}}
        private byte[] Device(out byte[] interfaces){
            var d=Control(new byte[]{128,6,0,1,0,0,18,0},new byte[0]);if(d.Length!=18||d[0]!=18||d[1]!=1)throw new IOException("Invalid firmware device descriptor");
            var header=Control(new byte[]{128,6,0,2,0,0,9,0},new byte[0]);if(header.Length!=9||header[1]!=2)throw new IOException("Invalid firmware configuration header");int length=header[2]|header[3]<<8;
            if(length<9||length>65535)throw new IOException("Invalid firmware configuration size");
            var config=Control(new byte[]{128,6,0,2,0,0,(byte)length,(byte)(length>>8)},new byte[0]);if(config.Length!=length)throw new IOException("Truncated firmware configuration");
            var info=new List<byte>();var ids=new HashSet<byte>();
            for(int i=0;i<config.Length;){int size=config[i];if(size<2||i+size>config.Length)throw new IOException("Malformed firmware descriptor");if(config[i+1]==4&&size>=9&&ids.Add(config[i+2]))info.AddRange(new byte[]{config[i+5],config[i+6],config[i+7],0});i+=size;}
            interfaces=info.ToArray();var block=new byte[312];System.Text.Encoding.ASCII.GetBytes("/n120/virtual/1-1").CopyTo(block,0);System.Text.Encoding.ASCII.GetBytes("1-1").CopyTo(block,256);Put(block,288,1);Put(block,292,1);Put(block,296,2);
            block[300]=d[9];block[301]=d[8];block[302]=d[11];block[303]=d[10];block[304]=d[13];block[305]=d[12];block[306]=d[4];block[307]=d[5];block[308]=d[6];block[309]=config[5];block[310]=d[17];block[311]=(byte)ids.Count;return block;
        }
        private static byte[] Management(ushort op,int status,bool list){var reply=new byte[list?12:8];reply[0]=1;reply[1]=17;reply[3]=(byte)op;Put(reply,4,(uint)status);return reply;}
        private sealed class Urb {public byte[] Header,Payload;public CancellationTokenSource Cancel;}
        private void Serve(TcpClient client){bool owns=false;try{using(client){
            var stream=client.GetStream();var h=Read(stream,8);ushort op=(ushort)(h[2]<<8|h[3]);if(h[0]!=1||h[1]!=17||U32(h,4)!=0||(op!=0x8005&&op!=0x8003))throw new IOException("Invalid USB/IP management request");
            string bus=op==0x8003?System.Text.Encoding.ASCII.GetString(Read(stream,32)).TrimEnd('\0'):"";
            lock(ownership){if(owner==null&&(op==0x8005||bus=="1-1")){owner=client;owns=true;}}
            if(!owns){var unavailable=Management(op,op==0x8003?1:0,op==0x8005);stream.Write(unavailable,0,unavailable.Length);return;}
            byte[] interfaces,device;
            try{device=Device(out interfaces);}catch(Exception e) when(e is IOException||e is OperationCanceledException){log("USB/IP enumeration: "+e.Message);var failed=Management(op,1,op==0x8005);stream.Write(failed,0,failed.Length);return;}
            var response=Management(op,0,op==0x8005);if(op==0x8005)Put(response,8,1);stream.Write(response,0,response.Length);stream.Write(device,0,device.Length);
            if(op==0x8005){stream.Write(interfaces,0,interfaces.Length);return;}
            lock(ownership)imported=true;client.ReceiveTimeout=0;
            // A dedicated receiver keeps UNLINK and disconnect responsive while
            // a bounded single worker serializes EP0 requests to the firmware.
            using(var ended=CancellationTokenSource.CreateLinkedTokenSource(stop.Token))
            using(var queue=new BlockingCollection<Urb>(64)){
                var pending=new Dictionary<uint,Urb>();var sync=new object();
                Action<byte[]> send=bytes=>{stream.Write(bytes,0,bytes.Length);};
                var executor=new Thread(()=>{try{foreach(var urb in queue.GetConsumingEnumerable(ended.Token)){
                    byte[] data=new byte[0];int status=0;bool input=U32(urb.Header,12)==1;
                    try{if(U32(urb.Header,16)!=0){status=-32;}else{var setup=new byte[8];Array.Copy(urb.Header,40,setup,0,8);int size=setup[6]|setup[7]<<8;
                        if(((setup[0]&128)!=0)!=input||size!=(int)U32(urb.Header,24))throw new IOException("USB/IP setup length/direction mismatch");
                        data=transfer(setup,urb.Payload,urb.Cancel.Token);if(data.Length>size)Array.Resize(ref data,size);
                    }}catch(OperationCanceledException){status=-110;}catch(IOException e){status=-32;log("USB/IP control: "+e.Message);}
                    lock(sync){uint sequence=U32(urb.Header,4);if(pending.Remove(sequence)){
                        var reply=new byte[48];Put(reply,0,3);Put(reply,4,sequence);Put(reply,20,unchecked((uint)status));Put(reply,24,status==0?(uint)(input?data.Length:urb.Payload.Length):0);send(reply);if(status==0&&input)send(data);
                    }}urb.Cancel.Dispose();
                }}catch(Exception e) when(e is OperationCanceledException||e is IOException||e is SocketException||e is ObjectDisposedException){ended.Cancel();client.Close();}}){IsBackground=true,Name="N120 USB/IP URB"};executor.Start();
                try{while(!ended.IsCancellationRequested){var packet=Read(stream,48);uint cmd=U32(packet,0),seq=U32(packet,4);
                    if(cmd==2){lock(sync){Urb target;bool found=pending.TryGetValue(U32(packet,20),out target);if(found){pending.Remove(U32(packet,20));target.Cancel.Cancel();}var reply=new byte[48];Put(reply,0,4);Put(reply,4,seq);Put(reply,20,found?unchecked((uint)-104):0);send(reply);}continue;}
                    if(cmd!=1||U32(packet,12)>1||U32(packet,24)>1048576||(U32(packet,32)!=0&&U32(packet,32)!=uint.MaxValue))throw new IOException("Unsupported or oversized USB/IP URB");
                    var urb=new Urb{Header=packet,Payload=U32(packet,12)==0?Read(stream,(int)U32(packet,24)):new byte[0],Cancel=CancellationTokenSource.CreateLinkedTokenSource(ended.Token)};urb.Cancel.CancelAfter(5000);
                    lock(sync){if(pending.ContainsKey(seq))throw new IOException("Duplicate USB/IP sequence");pending.Add(seq,urb);}
                    if(!queue.TryAdd(urb))throw new IOException("USB/IP queue full");
                }}finally{ended.Cancel();queue.CompleteAdding();client.Close();executor.Join(6000);lock(sync){foreach(var urb in pending.Values)urb.Cancel.Dispose();pending.Clear();}}
            }
        }}catch(EndOfStreamException){}catch(Exception e) when(e is IOException||e is SocketException||e is OperationCanceledException||e is ObjectDisposedException||e is InvalidOperationException){if(!stop.IsCancellationRequested)log("USB/IP: "+e.Message);}finally{
            if(owns){disconnect();lock(ownership)if(owner==client){owner=null;imported=false;}}
            lock(clients)clients.Remove(client);lock(workers)workers.Remove(Thread.CurrentThread);
        }}
        public void Dispose(){stop.Cancel();listener.Stop();accept.Join(1000);lock(clients)foreach(var client in clients)client.Close();Thread[] pending;lock(workers)pending=workers.ToArray();foreach(var thread in pending)if(thread!=Thread.CurrentThread)thread.Join(6500);}
    }

}
