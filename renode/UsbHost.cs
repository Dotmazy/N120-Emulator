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
        private readonly BlockingCollection<Action> operations=new BlockingCollection<Action>(128);
        private readonly Thread worker;public int Scheduled,Executed,Waits;
        public ReadyConnection(IUSBConnection connection,Func<bool> predicate,WaitHandle signal,object ioSync,bool endSessionOnDispose=true){IoSync=ioSync;inner=connection;ready=predicate;this.endSessionOnDispose=endSessionOnDispose;changed=signal;worker=new Thread(Run){IsBackground=true,Name="N120 USB host"};worker.Start();}
        private void Run(){try{foreach(var operation in operations.GetConsumingEnumerable(stop.Token)){while(true){stop.Token.ThrowIfCancellationRequested();lock(IoSync){if(ready()){operation();Executed++;break;}Waits++;}if(WaitHandle.WaitAny(new[]{changed,stop.Token.WaitHandle})==1)return;}}}catch(OperationCanceledException){}catch(ObjectDisposedException){}}
        public void Schedule(Action action){Scheduled++;operations.Add(action,stop.Token);}
        public void Flush(CancellationToken cancel){var done=new System.Threading.Tasks.TaskCompletionSource<bool>();Schedule(()=>done.TrySetResult(true));done.Task.Wait(cancel);}
        public IUSBPipeSetup ConnectEndpointSetup(byte ep){return new ReadySetup(inner.ConnectEndpointSetup(ep),this);}
        public IUSBPipeRead ConnectEndpointRead(byte ep){return inner.ConnectEndpointRead(ep);}
        public IUSBPipeWrite ConnectEndpointWrite(byte ep){return inner.ConnectEndpointWrite(ep);}
        public void Dispose(){stop.Cancel();operations.CompleteAdding();if(Thread.CurrentThread!=worker)worker.Join(1000);if(endSessionOnDispose)inner.Dispose();}
    }
    public sealed class ReadySetup:IUSBPipeSetup {
        private readonly IUSBPipeSetup pipe;private readonly ReadyConnection owner;
        public ReadySetup(IUSBPipeSetup p,ReadyConnection connection){pipe=p;owner=connection;}
        public event Action NewPacket {add{pipe.NewPacket+=value;}remove{pipe.NewPacket-=value;}}
        public bool TryRead(out byte[] data){lock(owner.IoSync)return pipe.TryRead(out data);}
        public void Write(byte[] data){var bytes=(byte[])data.Clone();owner.Schedule(()=>pipe.Write(bytes));}
        public void SetupPacketWrite(SetupPacket packet){owner.Schedule(()=>pipe.SetupPacketWrite(packet));}
    }
    // Bounded USB/IP endpoint-zero transport. Each host transfer is cancellable;
    // stopping closes accepted sockets as well as the listening socket.
    public sealed class LocalUsbIp:IDisposable {
        private readonly TcpListener listener;private readonly CancellationTokenSource stop=new CancellationTokenSource();
        private readonly List<TcpClient> clients=new List<TcpClient>();private readonly List<Thread> workers=new List<Thread>();private readonly Thread accept;
        private readonly Func<byte[],byte[],CancellationToken,byte[]> transfer;private readonly Action<string> log;
        public int Port {get;private set;}public int ClientCount {get{lock(clients)return clients.Count;}}
        public LocalUsbIp(int port,Func<byte[],byte[],CancellationToken,byte[]> transact,Action<string> diagnostics){transfer=transact;log=diagnostics;listener=new TcpListener(IPAddress.Loopback,port);listener.Start();Port=((IPEndPoint)listener.LocalEndpoint).Port;accept=new Thread(Accept){IsBackground=true,Name="N120 USB/IP accept"};accept.Start();}
        private void Accept(){try{while(!stop.IsCancellationRequested){var client=listener.AcceptTcpClient();client.NoDelay=true;lock(clients)clients.Add(client);var worker=new Thread(()=>Serve(client)){IsBackground=true,Name="N120 USB/IP client"};lock(workers)workers.Add(worker);worker.Start();}}catch(SocketException e){if(!stop.IsCancellationRequested)log(e.Message);}catch(ObjectDisposedException){}}
        private static byte[] Read(NetworkStream stream,int length){if(length<0||length>1048576)throw new IOException("USB/IP transfer exceeds 1 MiB");var result=new byte[length];int n=0;while(n<length){int got=stream.Read(result,n,length-n);if(got==0)throw new EndOfStreamException();n+=got;}return result;}
        private static uint U32(byte[] bytes,int offset){return (uint)(bytes[offset]<<24|bytes[offset+1]<<16|bytes[offset+2]<<8|bytes[offset+3]);}
        private static void Put(byte[] bytes,int offset,uint value){for(int i=0;i<4;i++)bytes[offset+i]=(byte)(value>>(24-8*i));}
        private byte[] Control(byte[] setup,byte[] data){using(var timeout=CancellationTokenSource.CreateLinkedTokenSource(stop.Token)){timeout.CancelAfter(3000);return transfer(setup,data,timeout.Token);}}
        private byte[] Device(out byte[] interfaces){
            var d=Control(new byte[]{128,6,0,1,0,0,18,0},new byte[0]);if(d.Length!=18)throw new IOException("Incomplete firmware device descriptor");
            var header=Control(new byte[]{128,6,0,2,0,0,9,0},new byte[0]);if(header.Length<9)throw new IOException("Incomplete firmware configuration header");int length=header[2]|header[3]<<8;
            var config=Control(new byte[]{128,6,0,2,0,0,(byte)length,(byte)(length>>8)},new byte[0]);var info=new List<byte>();var ids=new HashSet<byte>();
            for(int i=0;i+2<=config.Length&&config[i]>=2;i+=config[i])if(config[i+1]==4&&i+9<=config.Length&&ids.Add(config[i+2]))info.AddRange(new byte[]{config[i+5],config[i+6],config[i+7],0});
            interfaces=info.ToArray();var block=new byte[312];System.Text.Encoding.ASCII.GetBytes("/n120/virtual/1-1").CopyTo(block,0);System.Text.Encoding.ASCII.GetBytes("1-1").CopyTo(block,256);Put(block,288,1);Put(block,292,1);Put(block,296,2);
            block[300]=d[9];block[301]=d[8];block[302]=d[11];block[303]=d[10];block[304]=d[13];block[305]=d[12];block[306]=d[4];block[307]=d[5];block[308]=d[6];block[309]=config[5];block[310]=d[17];block[311]=(byte)ids.Count;return block;
        }
        private void Serve(TcpClient client){try{using(client){var stream=client.GetStream();var h=Read(stream,8);ushort op=(ushort)(h[2]<<8|h[3]);if(op!=0x8005&&op!=0x8003)throw new IOException("Unsupported USB/IP management request");if(op==0x8003)Read(stream,32);
            byte[] interfaces;var device=Device(out interfaces);var response=new byte[op==0x8005?12:8];response[0]=1;response[1]=17;response[3]=(byte)(op&255);if(op==0x8005)Put(response,8,1);stream.Write(response,0,response.Length);stream.Write(device,0,device.Length);
            if(op==0x8005){stream.Write(interfaces,0,interfaces.Length);return;}
            while(!stop.IsCancellationRequested){var packet=Read(stream,48);uint cmd=U32(packet,0);if(cmd==2){var unlink=new byte[48];Put(unlink,0,4);Array.Copy(packet,4,unlink,4,16);stream.Write(unlink,0,48);continue;}if(cmd!=1)throw new IOException("Unsupported USB/IP URB");uint requested=U32(packet,24);if(requested>1048576)throw new IOException("USB/IP transfer exceeds 1 MiB");int length=(int)requested;bool input=U32(packet,12)!=0;uint endpoint=U32(packet,16);var payload=input?new byte[0]:Read(stream,length);byte[] data=new byte[0];int status=0;
                try{if(endpoint!=0)throw new IOException("Only control endpoint zero is supported");var setup=new byte[8];Array.Copy(packet,40,setup,0,8);data=Control(setup,payload);if(input&&data.Length>length)Array.Resize(ref data,length);}catch(OperationCanceledException){status=-110;}catch(IOException){status=-32;}
                var reply=new byte[48];Put(reply,0,3);Array.Copy(packet,4,reply,4,16);Put(reply,20,unchecked((uint)status));Put(reply,24,(uint)(input?data.Length:payload.Length));stream.Write(reply,0,48);if(input)stream.Write(data,0,data.Length);
            }
        }}catch(Exception e) when(e is IOException||e is SocketException||e is OperationCanceledException||e is ObjectDisposedException||e is InvalidOperationException){if(!stop.IsCancellationRequested)log("USB/IP: "+e.Message);}finally{lock(clients)clients.Remove(client);lock(workers)workers.Remove(Thread.CurrentThread);}}
        public void Dispose(){stop.Cancel();listener.Stop();accept.Join(1000);lock(clients)foreach(var client in clients)client.Close();Thread[] pending;lock(workers)pending=workers.ToArray();foreach(var thread in pending)if(thread!=Thread.CurrentThread)thread.Join(1500);}
    }
}
