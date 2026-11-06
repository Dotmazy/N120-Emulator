// N120 board peripherals and register-level extensions for Renode.
using System;
using System.IO;
using System.Linq;
using System.Net;
using System.Net.Sockets;
using System.Text;
using System.Threading;
using System.Collections.Generic;
using Antmicro.Renode.Core;
using Antmicro.Renode.Peripherals;
using Antmicro.Renode.Peripherals.Bus;
using Antmicro.Renode.Peripherals.CPU;
using Antmicro.Renode.Peripherals.Memory;
using Antmicro.Renode.Peripherals.Timers;
using Antmicro.Renode.Peripherals.IRQControllers;
using System.Diagnostics;
using Antmicro.Renode.Time;
using Antmicro.Renode.Core.USB;

namespace Antmicro.Renode.Peripherals.N120
{
    // STM32 prescaler encodings 12..15 skip /32: /64,/128,/256,/512.
    // The generic 1.17 RCC model uses a continuous power-of-two table instead.
    public sealed class Rcc : IDoubleWordPeripheral,IPeripheral,IKnownSize {
        public Rcc(Board board,Nvic systick,Antmicro.Renode.Peripherals.Miscellaneous.STM32H7_RCC core){b=board;tick=systick;inner=core;}
        private readonly Antmicro.Renode.Peripherals.Miscellaneous.STM32H7_RCC inner;public long Size {get{return inner.Size;}}public uint ReadDoubleWord(long offset){return inner.ReadDoubleWord(offset);}
        public void Register(IPeripheral device,string alias){inner.Register(device,new Antmicro.Renode.Core.Structure.EnumRegistrationPoint<Antmicro.Renode.Peripherals.Miscellaneous.STM32H7_RCC.RegistrationPeripheralAlias>((Antmicro.Renode.Peripherals.Miscellaneous.STM32H7_RCC.RegistrationPeripheralAlias)Enum.Parse(typeof(Antmicro.Renode.Peripherals.Miscellaneous.STM32H7_RCC.RegistrationPeripheralAlias),alias)));}
        private readonly Board b;private readonly Nvic tick;
        public void WriteDoubleWord(long offset,uint value){inner.WriteDoubleWord(offset,value);if(offset<=0x40)UpdateClocks();}
        private static readonly ulong[] ahbDiv={1,1,1,1,1,1,1,1,2,4,8,16,64,128,256,512};
        private void UpdateClocks(){if(b==null)return;ulong core=b.CpuFrequency();if(core==0)return;tick.Frequency=core;ulong hclk=core/ahbDiv[inner.ReadDoubleWord(0x18)&15];uint apb=inner.ReadDoubleWord(0x1C);bool timpre=(inner.ReadDoubleWord(0x10)&(1u<<15))!=0;
            foreach(var pair in b.Timers){int shift=pair.Key==1||pair.Key==8?8:4;int bits=(int)((apb>>shift)&7);ulong div=bits<4?1u:1u<<(bits-3);ulong divisor=div<=2?1:Math.Max(1,div/(timpre?4u:2u));pair.Value.Frequency=hclk/divisor;}}
        public void Reset(){inner.Reset();UpdateClocks();}
    }
    // Renode 1.17 hardwires CLKSOURCE readback to one and ignores writes.
    // STM32H725 also provides the processor-clock / 8 SysTick reference input.
    public sealed class Nvic : NVIC,IDoubleWordPeripheral,IPeripheral {
        public Nvic(IMachine machine,ulong systickFrequency=64000000,byte priorityMask=0xF0,bool haltSystickOnDeepSleep=true):base(machine,systickFrequency,priorityMask,haltSystickOnDeepSleep){Divider=8;}
        private bool processorClock;
        public new uint ReadDoubleWord(long offset){uint value=base.ReadDoubleWord(offset);return offset==0x10?(value&~4u)|(processorClock?4u:0):value;}
        public new void WriteDoubleWord(long offset,uint value){if(offset==0x10){processorClock=(value&4)!=0;Divider=processorClock?1u:8u;}base.WriteDoubleWord(offset,value);}
        public new void Reset(){base.Reset();processorClock=false;Divider=8;}
    }
    public sealed class Usb : Antmicro.Renode.Peripherals.USB.STM_USB,IDoubleWordPeripheral,IUSBDevice,IPeripheral {
        public Usb(IMachine machine,Board board,Rcc clock):base(machine){b=board;b.Usb=this;clock.Register(this,"USB1OTGHS");base.IRQ.Connect(new InterruptJoin(this),0);recovery=new LimitTimer(machine.ClockSource,1000,this,"USB reset recovery",limit:10,eventEnabled:true,workMode:WorkMode.OneShot);recovery.LimitReached+=()=>{recoveryReady=true;SignalEndpointChanged();};}
        private readonly Board b;private uint phy,in0SizeEncoding,out0SizeEncoding;
        private readonly GPIO combinedIRQ=new GPIO();private bool baseInterrupt;private uint completedOut;
        public new GPIO IRQ {get{return combinedIRQ;}}
        private sealed class InterruptJoin:IGPIOReceiver {private readonly Usb owner;public InterruptJoin(Usb usb){owner=usb;}public void Reset(){owner.baseInterrupt=false;owner.UpdateIRQ();}public void OnGPIO(int number,bool value){owner.baseInterrupt=value;owner.UpdateIRQ();}}
        private bool ExtraOutPending(){return (completedOut&(base.ReadDoubleWord(0x81C)>>16))!=0&&(base.ReadDoubleWord(0x814)&1)!=0;}
        private void UpdateIRQ(){if(combinedIRQ!=null)combinedIRQ.Set(baseInterrupt||(ExtraOutPending()&&(base.ReadDoubleWord(8)&1)!=0&&(base.ReadDoubleWord(0x18)&(1u<<19))!=0));}
        private readonly uint[] endpointEvents=new uint[16];
        private IUSBConnection host,session;private bool pullupConnected,in0Stall,out0Stall;private readonly object ioSync=new object();
        private readonly AutoResetEvent endpointChanged=new AutoResetEvent(false),transferChanged=new AutoResetEvent(false);
        private void SignalEndpointChanged(){endpointChanged.Set();transferChanged.Set();}
        private readonly SemaphoreSlim hostGate=new SemaphoreSlim(1,1);
        private CancellationTokenSource connectionStop=new CancellationTokenSource();
        private readonly object connectionSync=new object();
        public volatile bool ProbePending;public long ConnectionGeneration;
        private readonly LimitTimer recovery;private bool recoveryReady=true;private uint rxFifoSize,txFifoSize,txFifoConfig;
        // USB 2.0 reset recovery: 10 ms of virtual USB time, independent of CPU clock.
        private void RecoverFromBusReset(){recoveryReady=false;recovery.Enabled=false;recovery.Value=0;recovery.Enabled=true;}
        // EP0 enable bits can survive a host reset. Wait for this reset ISR to
        // finish configuring FIFO banks before delivering the next SETUP packet.
        private long fifoGeneration,awaitedFifoGeneration,in0Generation,setupGeneration;
        private uint waitGlobal,waitControl,waitOut,waitInterrupt;
        private bool EndpointReady(){waitGlobal=base.ReadDoubleWord(0x14);waitControl=base.ReadDoubleWord(0x804);waitInterrupt=base.ReadDoubleWord(0xB08);waitOut=base.ReadDoubleWord(0xB00);return fifoGeneration>=awaitedFifoGeneration&&recoveryReady&&rxFifoSize>=2&&txFifoSize>=2&&(base.ReadDoubleWord(0x908)&1)==0&&(waitGlobal&0x3010)==0&&(waitControl&2)==0&&(waitOut&0x80008000)==0x80008000;}
        public new IUSBConnection ConnectUSB(){lock(ioSync){Array.Clear(endpointEvents,0,endpointEvents.Length);awaitedFifoGeneration=fifoGeneration+1;var connection=base.ConnectUSB();session=connection;RecoverFromBusReset();return new ReadyConnection(connection,EndpointReady,endpointChanged,ioSync,false){ReadReady=()=> (base.ReadDoubleWord(0x908)&1)==0,DataReady=()=> in0Stall||out0Stall||(base.ReadDoubleWord(0xB00)&(1u<<17))==0,Stalled=()=>in0Stall||out0Stall,BeforeSetup=()=>{in0Stall=out0Stall=false;setupGeneration++;SignalEndpointChanged();}};}}
        public long Transfers;public int Configuration;public volatile byte[] Descriptor=new byte[0];public volatile string LastError="";
        public void DisconnectHost(){
            // Token capture, cancellation and replacement share a lock: a probe
            // must not read Token from a source another host thread just disposed.
            lock(connectionSync)connectionStop.Cancel();
            SignalEndpointChanged();hostGate.Wait();
            try {
                if(host!=null){host.Dispose();host=null;}
                lock(connectionSync){connectionStop.Dispose();connectionStop=new CancellationTokenSource();}
                Descriptor=new byte[0];Configuration=0;ConnectionGeneration++;
            }finally{hostGate.Release();}
        }
        // Ending the data host is distinct from losing VBUS. Generic USBConnection.Dispose
        // raises OTG session-end, which is incorrect for Computer -> Charger or stopping USB/IP.
        public void EndSession(){DisconnectHost();var active=session;session=null;if(active!=null)lock(ioSync)active.Dispose();}
        public new void Reset(){lock(ioSync??this){pullupConnected=in0Stall=out0Stall=false;completedOut=0;rxFifoSize=txFifoSize=txFifoConfig=0;recoveryReady=true;if(recovery!=null)recovery.Enabled=false;base.Reset();phy=in0SizeEncoding=out0SizeEncoding=0;if(endpointEvents!=null)Array.Clear(endpointEvents,0,endpointEvents.Length);Descriptor=new byte[0];if(endpointChanged!=null)SignalEndpointChanged();UpdateIRQ();}}
        private void SyncAccess(){ICPU current;if(b!=null&&b.Machine.SystemBus.TryGetCurrentCPU(out current))current.SyncTime();}
        public new uint ReadDoubleWord(long o){SyncAccess();lock(ioSync){uint value=base.ReadDoubleWord(o)|(o==0xC?phy:0);if(o==0x900)value=(value&~0x2007FFu)|in0SizeEncoding|(in0Stall?1u<<21:0);if(o==0xB00)value=(value&~0x2007FFu)|out0SizeEncoding|(out0Stall?1u<<21:0);if(o>=0x908&&o<0xB00&&(o-0x908)%0x20==0)value|=endpointEvents[(o-0x908)/0x20];if(o==0x24)value=rxFifoSize;if(o==0x28)value=txFifoConfig;
            if(o>=0xB08&&o<0xD00&&(o-0xB08)%0x20==0&&(completedOut&(1u<<(int)((o-0xB08)/0x20)))!=0)value|=1;
            if(o==0x818)value|=completedOut<<16;if(o==0x14&&ExtraOutPending())value|=1u<<19;
            if(o==0x20&&((value>>17)&15)==2&&((value>>4)&0x7FF)==0){completedOut|=1u<<(int)(value&15);UpdateIRQ();}
            if(o==0x20||o>=0x1000)SignalEndpointChanged();return value;}}
        public new void WriteDoubleWord(long o,uint v){SyncAccess();lock(ioSync){if(o==0x910&&(v&(3u<<19))!=0)in0Generation++;if(o==0xC)phy=v&64;
            if(o>=0x900&&o<0xB00&&(o-0x900)%0x20==0){int ep=(int)((o-0x900)/0x20);if((v&(1u<<27))!=0)endpointEvents[ep]|=1u<<6;if((v&(1u<<30))!=0)endpointEvents[ep]|=1u<<1;}
            if(o>=0x908&&o<0xB00&&(o-0x908)%0x20==0)endpointEvents[(o-0x908)/0x20]&=~v;
            if(o>=0xB08&&o<0xD00&&(o-0xB08)%0x20==0&&(v&1)!=0)completedOut&=~(1u<<(int)((o-0xB08)/0x20));
            if(o==0x24){rxFifoSize=v&0xFFFF;SignalEndpointChanged();return;}
            if(o==0x28){txFifoConfig=v;txFifoSize=(v>>16)&0xFFFF;fifoGeneration++;SignalEndpointChanged();return;}
            if(o==0x10&&(v&1)!=0){uint config=base.ReadDoubleWord(0xC),power=base.ReadDoubleWord(0x38);base.Reset();base.WriteDoubleWord(0xC,config);base.WriteDoubleWord(0x38,power);Array.Clear(endpointEvents,0,endpointEvents.Length);completedOut=0;rxFifoSize=txFifoSize=txFifoConfig=0;pullupConnected=false;}
            if(o==0x14&&(v&0x3000)!=0&&session!=null&&b.ConnectionMode==2)RecoverFromBusReset();
            // EP0 MPSIZ is an STM32 2-bit encoding (0=64, 1=32,
            // 2=16, 3=8); Renode's generic endpoint expects a byte count.
            if(o==0x900){in0Stall=(v&(1u<<21))!=0;in0SizeEncoding=v&3;v=(v&~0x7FFu)|(64u>>(int)in0SizeEncoding);}
            if(o==0xB00){out0Stall=(v&(1u<<21))!=0;out0SizeEncoding=v&3;v=(v&~0x7FFu)|(64u>>(int)out0SizeEncoding);}
            base.WriteDoubleWord(o,v);
            if(o==0x804){bool connected=(v&2)==0;if(connected&&!pullupConnected&&session!=null&&b.ConnectionMode==2){awaitedFifoGeneration=fifoGeneration+1;session=base.ConnectUSB();RecoverFromBusReset();}pullupConnected=connected;}
            UpdateIRQ();SignalEndpointChanged();}
        }
        public byte[] Transfer(byte[] setup,byte[] payload,CancellationToken cancel){
            if(b.ConnectionMode!=2)throw new IOException("Computer mode / VBUS is required");
            if(b.Machine.IsPaused)throw new IOException("Emulation is paused; resume before a USB transfer");
            CancellationTokenSource linked;
            lock(connectionSync)linked=CancellationTokenSource.CreateLinkedTokenSource(cancel,connectionStop.Token);
            using(linked){
                hostGate.Wait(linked.Token);
                try{if(host==null)host=ConnectUSB();var pipe=host.ConnectEndpointSetup(0);
                    var packet=new SetupPacket{Recipient=(PacketRecipient)(setup[0]&31),Type=(PacketType)((setup[0]>>5)&3),Direction=(Antmicro.Renode.Core.USB.Direction)((setup[0]>>7)&1),Request=setup[1],Value=(ushort)(setup[2]|setup[3]<<8),Index=(ushort)(setup[4]|setup[5]<<8),Count=(ushort)(setup[6]|setup[7]<<8)};
                    byte[] result;
                    using(var received=new AutoResetEvent(false)){
                        Action notify=()=>{try{received.Set();}catch(ObjectDisposedException){}};pipe.NewPacket+=notify;
                        try {
                            long generation,setupEpoch;lock(ioSync){generation=in0Generation;setupEpoch=setupGeneration;}
                            pipe.SetupPacketWrite(packet);
                            // The generic pipe exposes EPENA from the preceding
                            // transaction until firmware services the SETUP IRQ.
                            // Wait for firmware to program this response's DIEPTSIZ0.
                            Action waitResponse=()=>{while(true){lock(ioSync){if(setupGeneration>setupEpoch){if(in0Stall||out0Stall)throw new IOException("Firmware STALL on endpoint zero");if(in0Generation>generation)break;}}WaitHandle.WaitAny(new[]{transferChanged,linked.Token.WaitHandle});linked.Token.ThrowIfCancellationRequested();}};
                            if((setup[0]&128)!=0){waitResponse();var data=new List<byte>();int maxPacket=64>>(int)(ReadDoubleWord(0x900)&3);
                                while(data.Count<packet.Count){byte[] part;while(!pipe.TryRead(out part)){WaitHandle.WaitAny(new[]{received,transferChanged,linked.Token.WaitHandle});linked.Token.ThrowIfCancellationRequested();}data.AddRange(part);if(part.Length<maxPacket)break;}
                                if(data.Count>packet.Count)data.RemoveRange(packet.Count,data.Count-packet.Count);result=data.ToArray();pipe.Write(new byte[0]);
                            }else{if(payload.Length>0)for(int i=0,packetSize=64>>(int)(ReadDoubleWord(0xB00)&3);i<payload.Length;i+=packetSize)pipe.Write(payload.Skip(i).Take(packetSize).ToArray());waitResponse();byte[] status;while(!pipe.TryRead(out status)){WaitHandle.WaitAny(new[]{received,transferChanged,linked.Token.WaitHandle});linked.Token.ThrowIfCancellationRequested();}result=new byte[0];}
                        }finally{pipe.NewPacket-=notify;}
                    }((ReadyConnection)host).Flush(linked.Token);Transfers++;if(setup[0]==0&&setup[1]==9)Configuration=setup[2];if(setup[0]==128&&setup[1]==6&&setup[3]==1)Descriptor=result;return result;
                }catch(OperationCanceledException){if(host!=null){host.Dispose();host=null;}Descriptor=new byte[0];Configuration=0;throw;}finally{hostGate.Release();}
            }
        }
        public string GetDescriptor(){
            try{using(var timeout=new CancellationTokenSource(3000)){var bytes=Transfer(new byte[]{128,6,0,1,0,0,18,0},new byte[0],timeout.Token);Descriptor=bytes;LastError="";return Convert.ToBase64String(bytes);}}
            catch(Exception e) when(e is IOException||e is OperationCanceledException){LastError=e is OperationCanceledException?"USB request cancelled or EP0 did not arm/complete; IN="+ReadDoubleWord(0x900).ToString("X8")+" OUT="+ReadDoubleWord(0xB00).ToString("X8")+" wait="+waitGlobal.ToString("X8")+","+waitControl.ToString("X8")+","+waitInterrupt.ToString("X8")+","+waitOut.ToString("X8")+" queue="+(host is ReadyConnection?((ReadyConnection)host).Scheduled+"/"+((ReadyConnection)host).Executed+"/"+((ReadyConnection)host).Waits:"none"):e.Message;return "";}
        }
        public void Probe(){if(ProbePending)return;ProbePending=true;ThreadPool.QueueUserWorkItem(_=>{try{GetDescriptor();}finally{ProbePending=false;}});}
        private LocalUsbIp server;public int UsbIpPort;
        public int UsbIpClients {get{var active=server;return active==null?0:active.ClientCount;}}public bool UsbIpImported {get{var active=server;return active!=null&&active.Imported;}}
        public void StartUsbIp(int port){if(server!=null)throw new ArgumentException("USB/IP already running");DisconnectHost();server=new LocalUsbIp(port,Transfer,DisconnectHost,message=>LastError=message);UsbIpPort=server.Port;}
        public void StopUsbIp(){var active=server;server=null;UsbIpPort=0;if(active!=null)active.Dispose();DisconnectHost();}

    }
    public sealed class Exti : STM32H7_EXTI,IDoubleWordPeripheral,IPeripheral {
        public Exti(IMachine machine):base(machine) {}
        private uint mask1,mask2,mask3;
        public new uint ReadDoubleWord(long o) {return o==0x84?mask1:o==0x94?mask2:o==0xA4?mask3:base.ReadDoubleWord(o);}
        public new void WriteDoubleWord(long o,uint v) {if(o==0x84)mask1=v;else if(o==0x94)mask2=v;else if(o==0xA4)mask3=v;else base.WriteDoubleWord(o,v);}
        public new void Reset() {base.Reset();mask1=mask2=mask3=0;}
    }
    public sealed class Board : IPeripheral, IDisposable
    {
        public Board(IMachine machine, CortexM cpu,Exti exti,MappedMemory internalFlash) { Machine = machine; Cpu = cpu; Exti=exti; InternalFlash=internalFlash;wfiHook=value=>CpuWaiting=value;Cpu.AddHookAtWfiStateChange(wfiHook); }
        private readonly Action<bool> wfiHook;public volatile bool CpuWaiting;
        private readonly MappedMemory InternalFlash;
        private uint bootAddress=0x08000000;
        public void ConfigureBoot(bool boot0){bootAddress=boot0?0x1FF00000u:0x08000000u;Cpu.InitVectorTableOffset=bootAddress;}
        private void ResetMcu(){Usb.StopUsbIp();bool resume=!Machine.IsPaused;Machine.Pause();try{((Machine)Machine).Reset();Cpu.InitVectorTableOffset=bootAddress;}finally{if(resume)Machine.Start();}}
        public int GdbPort;
        public void StartDebug(int port){if(port<1024||port>65535)throw new ArgumentException("GDB port must be 1024..65535");if(GdbPort!=0)throw new ArgumentException("Stop the existing GDB server first");Machine.StartGdbServer(port,true,(string)null);GdbPort=port;}
        public void StopDebug(){if(GdbPort!=0)Machine.StopGdbServer(GdbPort);GdbPort=0;}

        public readonly IMachine Machine;
        public readonly CortexM Cpu;
        public readonly Exti Exti;
        public Usb Usb;
        public readonly uint[] ExtiMux=new uint[4];
        public int BatteryPercent=70,ConnectionMode;
        private static readonly int[] pct={0,5,10,20,40,60,80,100};private static readonly double[] v={3.0,3.4,3.55,3.65,3.75,3.85,4.0,4.2};
        private double batteryVoltage=3.925;private uint batteryAdc=51445;
        public double BatteryVoltage {get{return batteryVoltage;}}
        public uint BatterySample {get{return batteryAdc;}}
        public long BatteryReads;public uint LastBatterySample;
        public uint ReadBatteryAdc(){BatteryReads++;return LastBatterySample=batteryAdc;}
        private static double VoltageFor(int percent){for(int i=1;i<pct.Length;i++)if(percent<=pct[i])return v[i-1]+(v[i]-v[i-1])*(percent-pct[i-1])/(pct[i]-pct[i-1]);return 4.2;}
        public double Backlight {get {var p=Ports[3];return p!=null&&p.Mode(3)==1&&(p.Odr&8)!=0?1:0;}}
        public void SetBattery(int percent) {if(percent<0||percent>100)throw new ArgumentException("Battery percentage out of range");lock(Sync){if(BatteryPercent==percent)return;BatteryPercent=percent;batteryVoltage=VoltageFor(percent);batteryAdc=(uint)Math.Round(Math.Min(65535,batteryVoltage/5*65535));}}
        public void SetConnection(int mode) {if(mode<0||mode>2)throw new ArgumentException("Invalid cable mode");bool before=ConnectionMode!=0;ConnectionMode=mode;if(mode!=2&&Usb!=null){Usb.StopUsbIp();if(mode==0)Usb.EndSession();else Usb.DisconnectHost();}if(before!=(mode!=0))SignalExti(9,mode!=0);}
        private readonly Stopwatch paceClock=new Stopwatch();
        private double paceEpoch,targetSpeed;
        private MasterTimeSource master;
        public void SetSpeed(double target) {
            if(target<0||target>16||double.IsNaN(target))throw new ArgumentException("Speed must be 0 (unlimited) or up to 16x");
            if(master==null){master=EmulationManager.Instance.CurrentEmulation.MasterTimeSource;master.SyncHook+=Pace;}
            targetSpeed=target;master.AdvanceImmediately=true;paceEpoch=master.ElapsedVirtualTime.TotalSeconds;paceClock.Restart();
        }
        private void Pace(TimeInterval time) {
            AdvanceSequence();
            if(targetSpeed<=0||!paceClock.IsRunning)return;
            double due=(master.ElapsedVirtualTime.TotalSeconds-paceEpoch)/targetSpeed-paceClock.Elapsed.TotalSeconds;
            if(due>0.001)Thread.Sleep((int)Math.Min(50,due*1000));
        }
        private readonly Queue<int> sequence=new Queue<int>();private int sequenceKey=-1;private double nextStroke;
        public void QueueSequence(string text) {lock(Sync){var strokes=text.Split(',').Select(int.Parse).ToArray();if(strokes.Any(x=>x<0||x>53)||sequence.Count+strokes.Length>512)throw new ArgumentException("Invalid or oversized key sequence");foreach(int i in strokes)sequence.Enqueue(i);if(master==null)SetSpeed(0);}}
        // Each queued contact and release spans two 20 ms firmware scan windows.
        private const double StrokeDuration=.04;
        private void AdvanceSequence() {lock(Sync){double now=Machine.ElapsedVirtualTime.TimeElapsed.TotalSeconds;if(now<nextStroke)return;if(sequenceKey>=0){Keys[sequenceKey/6,sequenceKey%6]=directKeys[sequenceKey/6,sequenceKey%6];sequenceKey=-1;UpdateColumns(true);nextStroke=now+StrokeDuration;}else if(sequence.Count>0){sequenceKey=sequence.Dequeue();Keys[sequenceKey/6,sequenceKey%6]=true;UpdateColumns(true);nextStroke=now+StrokeDuration;}}}
        public readonly object Sync = new object();
        public Lcd Display;
        public Ospi Flash;
        public readonly Gpio[] Ports = new Gpio[5];
        public readonly Dictionary<int,Timer> Timers = new Dictionary<int,Timer>();
        public readonly bool[,] Keys = new bool[9,6];
        private readonly bool[,] directKeys = new bool[9,6];
        public readonly Queue<string> Errors = new Queue<string>();
        public bool RevisedKeyboard = true;
        public long Accesses;
        public int[] RowPins = {1,0,2,3,6,7,8,10,15};
        public void Error(string message) { lock(Sync) { if(Errors.Count == 20) Errors.Dequeue(); Errors.Enqueue(message); } }
        public void Reset() { lock(Sync) { sequence.Clear();sequenceKey=-1;nextStroke=0;Array.Clear(Keys,0,Keys.Length);Array.Clear(directKeys,0,directKeys.Length);UpdateColumns(true); } }
        // Connectivity changes only on switch transitions. GPIO drive changes reuse
        // the same connected components, including all electrical ghost paths.
        private readonly uint[] components=new uint[15];
        private bool connectivityDirty=true,matrixDirty=true;
        private uint matrixRows=0xFFFF,matrixColumns=0xFFFF;
        private static readonly int[] OriginalColumns={1,2,3,4,5,6};
        private static readonly int[] RevisedColumns={1,9,11,4,5,6};
        public long MatrixSolves;
        public void InvalidateMatrix(bool switches=false){matrixDirty=true;if(switches)connectivityDirty=true;}
        private void ResolveMatrix(){
            if(!matrixDirty)return;
            MatrixSolves++;
            if(connectivityDirty){
                for(int i=0;i<15;i++)components[i]=1u<<i;
                for(int r=0;r<9;r++)for(int c=0;c<6;c++)if(Keys[r,c]){
                    uint joined=components[r]|components[9+c];
                    for(int i=0;i<15;i++)if((components[i]&joined)!=0)components[i]=joined;
                }
                connectivityDirty=false;
            }
            uint strongLow=0,weakLow=0,high=components[11];
            var a=Ports[0];var cp=Ports[2];var pins=RevisedKeyboard?RevisedColumns:OriginalColumns;
            for(int r=0;r<9;r++)if(a!=null){if(a.DrivesLow(RowPins[r]))strongLow|=components[r];if(a.PullDown(RowPins[r]))weakLow|=components[r];}
            for(int c=0;c<6;c++)if(cp!=null){if(cp.DrivesLow(pins[c]))strongLow|=components[9+c];if(cp.PullDown(pins[c]))weakLow|=components[9+c];}
            uint low=strongLow|(weakLow&~high);matrixRows=matrixColumns=0xFFFF;
            for(int r=0;r<9;r++)if((low&(1u<<r))!=0)matrixRows&=~(1u<<RowPins[r]);
            for(int c=0;c<6;c++)if((low&(1u<<(9+c)))!=0)matrixColumns&=~(1u<<pins[c]);
            matrixDirty=false;
        }
        public uint Columns(){ResolveMatrix();return matrixColumns;}
        public uint Rows(){ResolveMatrix();return matrixRows;}
        private uint lastColumns=0xFFFF;
        private uint lastRows=0xFFFF;
        public void UpdateColumns(bool switches=false) {
            InvalidateMatrix(switches);
            uint value=Columns(),rows=Rows();
            for(int pin=0;pin<16;pin++) {
                uint port=(ExtiMux[pin/4]>>(4*(pin%4)))&15;
                if(port==2 && ((value^lastColumns)&(1u<<pin))!=0)SignalExti(pin,(value&(1u<<pin))!=0);
                if(port==0 && ((rows^lastRows)&(1u<<pin))!=0)SignalExti(pin,(rows&(1u<<pin))!=0);
            }
            lastColumns=value;lastRows=rows;
        }
        private void SignalExti(int pin,bool high) {
            Exti.OnGPIO(pin,high);
            uint edge=Exti.ReadDoubleWord(high?0:4),events=Exti.ReadDoubleWord(0x84);
            if((edge&events&(1u<<pin))!=0)Cpu.SetEventFlag();
        }
        public double Led(int port,int pin)
        {
            var p=Ports[port];
            if(p==null) return 0;
            if(p.Mode(pin)==1) return (p.Odr & (1u<<pin))!=0?1:0;
            Timer timer;
            if(p.Mode(pin)==2 && port==1 && (pin==4||pin==5) && p.Alternate(pin)==2 && Timers.TryGetValue(3,out timer))return timer.Duty(pin-4);
            if(p.Mode(pin)==2 && port==2 && pin==8 && p.Alternate(pin)==3 && Timers.TryGetValue(8,out timer))return timer.Duty(2);
            if(p.Mode(pin)==2 && port==2 && pin==8 && p.Alternate(pin)==2 && Timers.TryGetValue(3,out timer))return timer.Duty(2);
            return 0;
        }
        public ulong CpuFrequency() {
            var bus=Machine.SystemBus;
            uint cr=bus.ReadDoubleWord(0x58024400),cfgr=bus.ReadDoubleWord(0x58024410);
            ulong hsi=64000000u>>(int)((cr>>3)&3),frequency=hsi;
            uint sourceMode=(cfgr>>3)&7;if(sourceMode==1)frequency=4000000;if(sourceMode==2)frequency=8000000;
            if(sourceMode==3) {
                uint sel=bus.ReadDoubleWord(0x58024428),div=bus.ReadDoubleWord(0x58024430);
                uint m=(sel>>4)&63;
                if(m==0)return 0;
                ulong source=(sel&3)==0?hsi:(sel&3)==1?4000000u:8000000u;
                frequency=source*((div&511)+1)/m/(((div>>9)&127)+1);
            }
            int pre=(int)((bus.ReadDoubleWord(0x58024418)>>8)&15);
            int[] divisors={1,1,1,1,1,1,1,1,2,4,8,16,64,128,256,512};
            return frequency/(ulong)divisors[pre];
        }
        public string Snapshot()
        {
            var pc=Cpu.PC.RawValue;var sp=Cpu.SP.RawValue;
            lock(Sync) {
                var pixels=new byte[320*240*2];
                if(Display!=null && Display.Active) Buffer.BlockCopy(Display.Pixels,0,pixels,0,pixels.Length);
                return "{\"ok\":true,\"cpuWaiting\":"+(CpuWaiting?"true":"false")+",\"pc\":"+pc+",\"sp\":"+sp+
                    ",\"paused\":"+(Machine.IsPaused?"true":"false")+",\"faulted\":"+(Cpu.IsLockedUp?"true":"false")+
                    ",\"instructions\":"+Cpu.ExecutedInstructions+",\"virtualMs\":"+Machine.ElapsedVirtualTime.TimeElapsed.TotalMilliseconds.ToString(System.Globalization.CultureInfo.InvariantCulture)+
                    ",\"cpuHz\":"+CpuFrequency()+",\"backlight\":"+Number(Backlight)+",\"batteryPercent\":"+BatteryPercent+",\"batteryVoltage\":"+Number(BatteryVoltage)+",\"batteryAdc\":"+BatterySample+",\"batteryReads\":"+BatteryReads+",\"lastBatterySample\":"+LastBatterySample+",\"connectionMode\":"+ConnectionMode+",\"targetSpeed\":"+Number(targetSpeed)+",\"sequencePending\":"+sequence.Count+
                    ",\"rows\":"+Rows()+
                    ",\"cfsr\":"+Machine.SystemBus.ReadDoubleWord(0xE000ED28)+",\"hfsr\":"+Machine.SystemBus.ReadDoubleWord(0xE000ED2C)+
                    ",\"usbIn0\":"+(Usb==null?0:Usb.ReadDoubleWord(0x900))+",\"usbOut0\":"+(Usb==null?0:Usb.ReadDoubleWord(0xB00))+",\"usbIpPort\":"+(Usb==null?0:Usb.UsbIpPort)+",\"usbClients\":"+(Usb==null?0:Usb.UsbIpClients)+",\"usbImported\":"+(Usb!=null&&Usb.UsbIpImported?"true":"false")+",\"usbConfiguration\":"+(Usb==null?0:Usb.Configuration)+",\"gdbPort\":"+GdbPort+",\"gdbConnected\":"+(GdbPort!=0&&Machine.IsGdbConnectedToServer(GdbPort)?"true":"false")+",\"bootAddress\":"+bootAddress+
                    ",\"pressed\":["+string.Join(",",Enumerable.Range(0,54).Where(i=>Keys[i/6,i%6]))+"],\"usbTransfers\":"+(Usb==null?0:Usb.Transfers)+",\"usbAddress\":"+(Usb==null?0:Usb.Address)+",\"usbDescriptor\":\""+(Usb==null?"":Convert.ToBase64String(Usb.Descriptor))+"\""+
                    ",\"lcdId\":"+(Display==null?0:Display.Id)+",\"lcdActive\":"+(Display!=null && Display.Active?"true":"false")+
                    ",\"gamma\":"+(Display!=null && Display.GammaValid?"true":"false")+
                    ",\"pixelWrites\":"+(Display==null?0:Display.PixelWrites)+",\"accesses\":"+Accesses+
                    ",\"rgb\":["+Number(Led(1,4))+","+Number(Led(1,5))+","+Number(Led(2,8))+"],\"columns\":"+Columns()+
                    ",\"flashWrites\":"+(Flash==null?0:Flash.ProgramBytes)+",\"flashErases\":"+(Flash==null?0:Flash.Erases)+
                    ",\"flashStatus\":"+(Flash==null?0:Flash.Status)+
                    ",\"errors\":["+string.Join(",",Errors.Select(x=>"\""+JsonEscape(x)+"\""))+
                    "],\"usbPending\":"+(Usb!=null&&Usb.ProbePending?"true":"false")+",\"usbError\":\""+(Usb==null?"":JsonEscape(Usb.LastError))+"\",\"frame\":\""+Convert.ToBase64String(pixels)+"\"}";
            }
        }
        internal static byte[] ReadImage(string path,int limit) {
            using(var input=File.OpenRead(path)) {
                if(input.Length>limit)throw new ArgumentException("Image exceeds its mapped capacity");
                var bytes=new byte[(int)input.Length];int offset=0;
                while(offset<bytes.Length) {
                    int count=input.Read(bytes,offset,bytes.Length-offset);
                    if(count==0)throw new IOException("Image changed or was truncated while reading");
                    offset+=count;
                }
                if(input.ReadByte()!=-1)throw new IOException("Image grew while reading");
                return bytes;
            }
        }
        public void Reload(string internalPath,string externalPath,int offset,string persistentPath) {
            var image=ReadImage(internalPath,0x80000);if(image.Length<8||image.Length>0x80000)throw new ArgumentException("Internal image must fit 512 KiB");
            uint sp=BitConverter.ToUInt32(image,0),pc=BitConverter.ToUInt32(image,4);if((pc&1)==0||pc<0x08000001||pc>=0x08000000+image.Length||(sp&7)!=0||!((sp>0x20000000&&sp<=0x20020000)||(sp>0x24000000&&sp<=0x24050000)||(sp>0x30000000&&sp<=0x30008000)||(sp>0x38000000&&sp<=0x38004000)))throw new ArgumentException("Invalid reset vectors");
            if(offset<0||offset>0x800000)throw new ArgumentException("Invalid external image offset");
            var external=externalPath==""?new byte[0]:ReadImage(externalPath,0x800000-offset);if(offset<0||external.Length>0x800000-offset)throw new ArgumentException("External image exceeds 8 MiB");
            var persistent=persistentPath==""?new byte[0]:ReadImage(persistentPath,0x800000);if(persistent.Length>0x800000)throw new ArgumentException("Persistent image exceeds 8 MiB");
            bool resume=!Machine.IsPaused;Usb.StopUsbIp();Machine.Pause();
            try {((Machine)Machine).Reset();var erased=Enumerable.Repeat((byte)255,0x80000).ToArray();InternalFlash.WriteBytes(0,erased);InternalFlash.WriteBytes(0,image);Flash.Replace(external,offset,persistent);Display.ConfigureRevision(image.Length>=0x7FA04?BitConverter.ToUInt32(image,0x7FA00):uint.MaxValue);Reset();Cpu.InitVectorTableOffset=bootAddress;}
            finally {if(resume){if(master!=null){paceEpoch=master.ElapsedVirtualTime.TotalSeconds;paceClock.Restart();}Machine.Start();}}
        }
        private static string JsonEscape(string value) {return value.Replace("\\","\\\\").Replace("\"","\\\"").Replace("\r","\\r").Replace("\n","\\n").Replace("\t","\\t");}
        private static string Number(double value) {return value.ToString(System.Globalization.CultureInfo.InvariantCulture);}
        // Bound command lines so a local client cannot grow an unbounded ReadLine buffer.
        private static string ReadCommand(StreamReader reader) {
            var line=new StringBuilder();
            int next;
            while((next=reader.Read())!=-1) {
                if(next=='\n')return line.ToString().TrimEnd('\r');
                if(line.Length>=65536)throw new IOException("Bridge command exceeds 64 KiB");
                line.Append((char)next);
            }
            return line.Length==0?null:line.ToString();
        }
        public void StartBridge(int port)
        {
            if(listener!=null)throw new ArgumentException("Bridge is already listening");
            var server=new TcpListener(IPAddress.Loopback,port);
            server.Start();listener=server;
            thread=new Thread(()=> {
                try { while(listener!=null) {
                    try { using(var client=server.AcceptTcpClient()) {
                        bridgeClient=client;client.NoDelay=true;
                        using(var reader=new StreamReader(client.GetStream(),Encoding.UTF8))
                        using(var writer=new StreamWriter(client.GetStream(),new UTF8Encoding(false)) {AutoFlush=true}) {
                            string line;
                            while((line=ReadCommand(reader))!=null) {
                                try {
                                    writer.WriteLine(Command(line));
                                    if(line=="shutdown") { Usb.StopUsbIp();Machine.Pause(); ((Machine)Machine).Dispose(); Environment.Exit(0); }
                                } catch(Exception e) when(e is ArgumentException || e is OverflowException || e is FormatException || e is IOException || e is UnauthorizedAccessException || e is SocketException) {
                                    Error(e.GetType().Name+": "+e.Message);
                                    writer.WriteLine("{\"ok\":false,\"error\":\""+JsonEscape(e.Message)+"\"}");
                                }
                            }
                        }
                    }} catch(SocketException e) {if(listener!=null)Error(e.Message);}
                    catch(IOException e) {if(listener!=null)Error(e.Message);}
                    finally {bridgeClient=null;}
                }} catch(ObjectDisposedException) {}
            }); thread.IsBackground=true;thread.Name="N120 GUI bridge";thread.Start();
        }
        private string Command(string line)
        {
            var p=line.Split(' ');
            int arguments;
            switch(p[0]) {
            case "reload": arguments=5;break;
            case "key": arguments=4;break;
            case "battery":case "cable":case "speed":case "sequence":case "usbip":case "gdb-start":case "save":arguments=2;break;
            default:arguments=1;break;
            }
            if(p.Length!=arguments)throw new ArgumentException("Invalid bridge command arguments");
            if(p[0]=="snapshot") return Snapshot();
            if(p[0]=="shutdown") return "{\"ok\":true}";
            if(p[0]=="reload") {Reload(Encoding.UTF8.GetString(Convert.FromBase64String(p[1])),p[2]=="-"?"":Encoding.UTF8.GetString(Convert.FromBase64String(p[2])),int.Parse(p[3]),p[4]=="-"?"":Encoding.UTF8.GetString(Convert.FromBase64String(p[4])));return "{\"ok\":true,\"reloaded\":true}";}
            if(p[0]=="reset") { ResetMcu();return "{\"ok\":true}"; }
            if(p[0]=="pause") Machine.Pause();
            else if(p[0]=="resume") {if(master!=null){paceEpoch=master.ElapsedVirtualTime.TotalSeconds;paceClock.Restart();}Machine.Start();}
            else if(p[0]=="battery") SetBattery(int.Parse(p[1]));
            else if(p[0]=="cable") SetConnection(int.Parse(p[1]));
            else if(p[0]=="speed") SetSpeed(double.Parse(p[1],System.Globalization.CultureInfo.InvariantCulture));
            else if(p[0]=="sequence") QueueSequence(p[1]);
            else if(p[0]=="usb-descriptor") return "{\"ok\":true,\"usbDescriptor\":\""+Usb.GetDescriptor()+"\"}";
            else if(p[0]=="usb-probe") Usb.Probe();
            else if(p[0]=="usbip") Usb.StartUsbIp(int.Parse(p[1]));
            else if(p[0]=="usbip-stop") Usb.StopUsbIp();
            else if(p[0]=="gdb-start") StartDebug(int.Parse(p[1]));
            else if(p[0]=="gdb-stop") StopDebug();
            else if(p[0]=="key" && p.Length==4) { lock(Sync) {int r=int.Parse(p[1]),c=int.Parse(p[2]);if(r<0||r>=9||c<0||c>=6||(p[3]!="0"&&p[3]!="1"))throw new ArgumentException("Invalid matrix key");directKeys[r,c]=p[3]=="1";Keys[r,c]=directKeys[r,c]||sequenceKey==r*6+c;UpdateColumns(true);} }
            else if(p[0]=="release") Reset();
            else if(p[0]=="save" && Flash!=null) Flash.Save(Encoding.UTF8.GetString(Convert.FromBase64String(p[1])));
            else if(p[0]!="pause") throw new ArgumentException("Unknown bridge command");
            return "{\"ok\":true}";
        }
        public void Dispose() {StopDebug();Usb.StopUsbIp();Cpu.RemoveHookAtWfiStateChange(wfiHook); if(master!=null)master.SyncHook-=Pace;var l=listener; listener=null; if(l!=null) l.Stop();var client=bridgeClient;if(client!=null)client.Close();if(thread!=null&&thread!=Thread.CurrentThread)thread.Join(1000); }
        private volatile TcpClient bridgeClient;
        private volatile TcpListener listener;
        private Thread thread;
    }

    public sealed class Gpio : IDoubleWordPeripheral, IKnownSize
    {
        public Gpio(Board board,int port) { b=board; index=port; b.Ports[port]=this; Reset(); }
        public long Size { get { return 0x400; } }
        public uint Odr { get { return regs[5]; } }
        public int Mode(int pin) { return (int)((regs[0]>>(pin*2))&3); }
        public int Alternate(int pin) {return (int)((regs[8+pin/8]>>(4*(pin%8)))&15);}
        public bool DrivesLow(int pin) {return Mode(pin)==1&&(Odr&(1u<<pin))==0;}
        public bool PullDown(int pin) {return Mode(pin)!=3&&((regs[3]>>(2*pin))&3)==2;}
        public uint ReadDoubleWord(long offset)
        {
            lock(b.Sync) {
                b.Accesses++;
                if(offset==0x10) {
                    uint value=index==2?b.Columns():index==0?b.Rows():0xFFFF;
                    if(index==1 && b.Display!=null && b.Display.Active) {
                        b.Cpu.SyncTime();
                        bool te=b.Machine.ElapsedVirtualTime.TimeElapsed.TotalMilliseconds% (1000.0/60.0) < 1;
                        value=(value&~2u)|(te?2u:0);
                    }
                    if(index==0)value=(value&~(1u<<9))|(b.ConnectionMode!=0?1u<<9:0);
                    if(index==2)value=(value&~(1u<<10))|(b.ConnectionMode!=0&&b.BatteryPercent<100?0:1u<<10); // Charger CHG_SB is open-drain active low on PC10.
                    for(int pin=0;pin<16;pin++) if(Mode(pin)==1) value=(value&~(1u<<pin))|(Odr&(1u<<pin));
                    return value;
                }
                return regs[offset/4];
            }
        }
        public void WriteDoubleWord(long offset,uint value)
        {
            lock(b.Sync) { b.Accesses++; if(offset==0x18) regs[5]=(regs[5]|(value&0xFFFF)) & ~(value>>16);
                else if(offset==0x28) regs[5] &= ~value; else if(offset!=0x10) regs[offset/4]=value; if((index==0||index==2)&&(offset==0||offset==0x0C||offset==0x14||offset==0x18||offset==0x28))b.UpdateColumns(); }
        }
        public void Reset() { Array.Clear(regs,0,regs.Length); regs[0]=0xFFFFFFFF;b.InvalidateMatrix(); }
        private readonly uint[] regs=new uint[256];
        private readonly Board b;
        private readonly int index;
    }

    public sealed class Timer : STM32_Timer, IDoubleWordPeripheral, IWordPeripheral, IPeripheral
    {
        public Timer(IMachine machine,Board board,int number,Rcc clock) : base(machine,270000000,number==2?0xFFFFFFFFu:0xFFFFu) {b=board;id=number;b.Timers[number]=this;clock.Register(this,"TIM"+number);LimitReached+=()=>Interlocked.Increment(ref wraps);}
        public new uint ReadDoubleWord(long o) {double counter;bool gate;return o==0x24&&GatedCounter(out counter,out gate)?(uint)counter:base.ReadDoubleWord(o);}
        public new void WriteDoubleWord(long o,uint value) {lock(b.Sync) {if(o<0x100)values[o/4]=value;base.WriteDoubleWord(o,value);if(o==8||o==0x24||(o==0x14&&(value&1)!=0))CaptureGateOrigin();}}
        public ushort ReadWord(long o) {return (ushort)(ReadDoubleWord(o&~3)>>(8*(int)(o&3)));}
        public void WriteWord(long o,ushort value) {long a=o&~3;int shift=8*(int)(o&3);WriteDoubleWord(a,(ReadDoubleWord(a)&~(0xFFFFu<<shift))|((uint)value<<shift));}
        public new void Reset() {base.Reset();wraps=0;gateEpoch=gateInitial=0;if(values!=null)Array.Clear(values,0,values.Length);}
        private long wraps;private double gateEpoch,gateInitial;
        public uint ProgrammedControl2 {get{return values[1];}}
        private Timer GateMaster(){Timer master;return id==3&&(values[2]&0x77)==5&&b.Timers.TryGetValue(1,out master)?master:null;}
        private bool HighTicks(out double cumulative,out bool high){
            int output=(int)((values[1]>>4)&7)-4;cumulative=0;high=false;if(output<0||output>3)return false;
            int mode=(int)((values[(output<2?0x18:0x1C)/4]>>(4+8*(output%2)))&7);if(mode!=6&&mode!=7)return false;
            double period=values[0x2C/4]+1.0,compare=Math.Min(period,values[(0x34+output*4)/4]),counter=Value;
            double perCycle=mode==6?compare:period-compare,partial=mode==6?Math.Min(counter,compare):Math.Max(0,counter-compare);
            cumulative=Interlocked.Read(ref wraps)*perCycle+partial;high=(values[0]&1)!=0&&(mode==6?counter<compare:counter>=compare);return true;
        }
        private void CaptureGateOrigin(){var master=GateMaster();double ticks;bool high;gateInitial=base.ReadDoubleWord(0x24);gateEpoch=master!=null&&master.HighTicks(out ticks,out high)?ticks:0;}
        // TIM3 ITR0 is TIM1 TRGO. Integrate the master's qualified counter ticks;
        // this preserves the stopped slave phase between exam-LED PWM bursts.
        private bool GatedCounter(out double counter,out bool gate){counter=0;gate=false;var master=GateMaster();double ticks;if(master==null||!master.HighTicks(out ticks,out gate))return false;
            double ratio=Frequency/(double)Divider/(master.Frequency/(double)master.Divider);counter=(gateInitial+Math.Max(0,ticks-gateEpoch)*ratio)%(values[0x2C/4]+1.0);return true;}
        public double Duty(int ch) {
            if( (values[0x20/4]&(1u<<(ch*4)))==0 || (id==8&&(values[0x44/4]&(1u<<15))==0))return 0;
            int mode=(int)((values[(ch<2?0x18:0x1C)/4]>>(4+8*(ch%2)))&7);
            if(mode==4||mode==5){double forced=mode==5?1:0;return (values[0x20/4]&(2u<<(ch*4)))!=0?1-forced:forced;}
            if((values[0]&1)==0)return 0;
            double duty=Math.Min(1.0,(double)values[(0x34+ch*4)/4]/(values[0x2C/4]+1.0));
            if(mode==6||mode==7){double counter;bool gate;bool gated=GatedCounter(out counter,out gate);double hz=Frequency/(double)Divider/(values[0x2C/4]+1.0);if(hz<30||(gated&&!gate))duty=(gated?counter:Value)<values[(0x34+ch*4)/4]?1:0;}
            if(mode==7)duty=1-duty;else if(mode!=6)return 0;
            if((values[0x20/4]&(2u<<(ch*4)))!=0)duty=1-duty;
            return duty;
        }
        private readonly Board b;private readonly int id;private readonly uint[] values=new uint[64];
    }

    public sealed class Lcd : IWordPeripheral, IBytePeripheral, IDoubleWordPeripheral, IKnownSize
    {
        public void StartBridge(int port) { b.StartBridge(port); }
        public string Snapshot() { return b.Snapshot(); }
        public void ConfigureBoot(bool boot0){b.ConfigureBoot(boot0);}
        public void StartDebug(int port){b.StartDebug(port);}
        public void StopDebug(){b.StopDebug();}
        public void ConfigureRevision(uint encoded) { b.RevisedKeyboard = encoded!=0 && ~encoded>=420;b.InvalidateMatrix(); }
        public void ConfigureBattery(int percent) {b.SetBattery(percent);}
        public Lcd(Board board,uint id=0x4E4101) { b=board; b.Display=this; Id=id; expected=Expected(id); Reset(); }
        public uint Id {get; private set;}
        public long Size {get {return 0x20004;} }
        public readonly ushort[] Pixels=new ushort[320*240];
        public long PixelWrites;
        private readonly byte[][] expected;
        private bool validationDirty=true,gammaValid,active;
        private void Validate(){if(!validationDirty)return;gammaValid=Same(0xE0,expected[0])&&Same(0xE1,expected[1]);active=awake&&on&&gammaValid&&(Value(0x3A)==0x55||Value(0x3A)==5)&&Value(0x35)==0&&inversion&&PowerValid;validationDirty=false;}
        public bool GammaValid {get {Validate();return gammaValid;} }
        public bool Active {get {Validate();return active;} }
        private bool PowerValid {get {
            int rate=(Id==0x4E4101||Id==0x4E4105||Id==0x4E4106||Id==0x4E4801)?0x1E:5;
            if(Value(0xC6)!=rate) return false;
            if(Id!=0x4E4102 && Id!=0x4E5401) return true;
            return Same(0xB2,new byte[]{12,12,0,0x33,0x33}) && Value(0xB7)==0x35 && Value(0xBB)==0x3C &&
                Value(0xC0)==0x2C && Value(0xC2)==1 && Value(0xC3)==0x10 && Value(0xC4)==0x20 && Same(0xD0,new byte[]{0xA4,0xA1});
        } }
        private int Value(int cmd) { List<byte> v; return config.TryGetValue(cmd,out v) && v.Count==1 ? v[0]:-1; }
        private bool Same(int cmd,byte[] expected) {List<byte> v; return config.TryGetValue(cmd,out v) && v.SequenceEqual(expected); }
        public static byte[][] Expected(uint id) {
            byte[] a;
            switch(id) {
            case 0x4E4101: case 0x4E4105: case 0x4E4106: case 0x4E4801: a=new byte[]{0xA2,10,17,10,12,26,52,34,77,40,21,19,41,45}; break;
            case 0x4E4102: a=new byte[]{0xF0,9,15,8,7,3,40,17,60,23,18,17,40,45}; break;
            case 0x4E4601: a=new byte[]{0xC4,12,19,12,14,60,54,68,79,58,23,21,43,47}; break;
            case 0x4E5401: return new[]{new byte[]{0xF0,13,18,9,8,20,45,34,67,56,23,25,44,48},new byte[]{0xF0,9,13,10,9,5,41,67,64,57,24,22,46,46}};
            default: throw new ArgumentException("Unsupported LCD identity");
            }
            return new[]{a,a};
        }
        public void Reset() { lock(b.Sync) {config.Clear(); validationDirty=true;awake=on=inversion=false; command=0; readIndex=0; x0=y0=x=y=0; x1=319; y1=239; Array.Clear(Pixels,0,Pixels.Length); PixelWrites=0;} }
        public void WriteWord(long offset,ushort value)
        {
            lock(b.Sync) {
                b.Accesses++;
                if(offset==0) {
                    command=value&255; readIndex=0;validationDirty=true;
                    readByteIndex=0;
                    if(!new int[]{1,4,0x10,0x11,0x20,0x21,0x28,0x29,0x2A,0x2B,0x2C,0x2E,0x35,0x36,0x3A,0xB2,0xB7,0xBB,0xC0,0xC2,0xC3,0xC4,0xC6,0xD0,0xE0,0xE1}.Contains(command))b.Error("Unhandled LCD command "+command.ToString("X2"));
                    if(command==1) {Reset(); return;}
                    if(command==0x11) awake=true;
                    if(command==0x10) awake=false;
                    if(command==0x29) {on=true; if(!Active) b.Error("LCD display-on rejected: gamma or configuration incomplete");}
                    if(command==0x28) on=false;
                    if(command==0x21) inversion=true;
                    if(command==0x20) inversion=false;
                    if(command==0x2C || command==0x2E) {x=x0;y=y0;}
                    else config[command]=new List<byte>();
                    return;
                }
                if(offset!=0x20000) {b.Error("LCD invalid bus offset");return;}
                if(command==0x2C) {
                    if(Active) {int px,py; Coordinates(out px,out py); if(px>=0 && px<320 && py>=0 && py<240) {Pixels[py*320+px]=value;PixelWrites++;} }
                    Advance(); return;
                }
                List<byte> data; if(!config.TryGetValue(command,out data)) config[command]=data=new List<byte>();
                if(data.Count<256) data.Add((byte)value);if(command!=0x2A&&command!=0x2B&&command!=0x36)validationDirty=true;
                if((command==0x2A||command==0x2B) && data.Count==4) {
                    int first=(data[0]<<8)|data[1],last=(data[2]<<8)|data[3];
                    if(command==0x2A) {x0=first;x1=last;} else {y0=first;y1=last;}
                }
            }
        }
        private void Coordinates(out int px,out int py) {
            int mad=Value(0x36);
            // MCU native column/row addressing is portrait; reference uses 0xA0 for landscape.
            if((mad&0x20)!=0) {px=x;py=y;} else {px=319-y;py=x;}
        }
        private void Advance() { if(++x>x1) {x=x0;if(++y>y1)y=y0;} }
        public ushort ReadWord(long offset) {
            lock(b.Sync) {b.Accesses++;
                if(offset!=0x20000)return 0;
                if(command==4) {int i=readIndex++;return (ushort)(i==0?0:i<=3?(Id>>(8*(3-i)))&255:0);}
                if(command==0x2E) {
                    if(readIndex++==0) return 0;
                    // RGB666 read stream: three halfwords encode two pixels.
                    ushort result=0;
                    for(int i=0;i<2;i++) {
                        int px,py;Coordinates(out px,out py);ushort p=px>=0&&px<320&&py>=0&&py<240?Pixels[py*320+px]:(ushort)0;
                        int channel=(readByteIndex++)%3;
                        byte v=(byte)(channel==0?(p>>11)<<3:channel==1?((p>>5)&63)<<2:(p&31)<<3);
                        result=(ushort)((result<<8)|v); if(channel==2)Advance();
                    }
                    return result;
                }
                b.Error("Unhandled LCD read command "+command.ToString("X2")); return 0;
            }
        }
        public byte ReadByte(long o) {return (byte)ReadWord(o);}
        public uint ReadDoubleWord(long o) {return ReadWord(o);}
        public void WriteByte(long o,byte v) {WriteWord(o,v);}
        public void WriteDoubleWord(long o,uint v) {WriteWord(o,(ushort)v);}
        private readonly Board b;
        private readonly Dictionary<int,List<byte>> config=new Dictionary<int,List<byte>>();
        private int command,readIndex,readByteIndex,x0,x1,y0,y1,x,y;
        private bool awake,on,inversion;
    }

    public sealed class Ospi : IDoubleWordPeripheral, IBytePeripheral, IWordPeripheral, IKnownSize
    {
        public Ospi(Board board,MappedMemory memory) {b=board; m=memory; b.Flash=this; Reset();}
        public void ProtectMapping() {
            var bus=(SystemBus)b.Machine.SystemBus;
            bus.SetHookBeforePeripheralWrite<byte>(m,(v,o)=>m.ReadByte(o),null);
            bus.SetHookBeforePeripheralWrite<ushort>(m,(v,o)=>m.ReadWord(o),null);
            bus.SetHookBeforePeripheralWrite<uint>(m,(v,o)=>m.ReadDoubleWord(o),null);
            bus.SetHookBeforePeripheralWrite<ulong>(m,(v,o)=>m.ReadQuadWord(o),null);
        }
        public long Size {get{return 0x400;}}
        public long ProgramBytes,Erases;
        public byte Status {get {return (byte)((wel?2:0)|(busyReads>0?1:0));}}
        public void Load(string path,int offset=0) {
            if(offset<0||offset>0x800000)throw new ArgumentException("Invalid flash offset");
            var bytes=Board.ReadImage(path,0x800000-offset); if(offset<0 || bytes.Length>0x800000-offset)throw new ArgumentException("Flash image exceeds 8 MiB");
            m.WriteBytes(offset,bytes); }
        public void Replace(byte[] external,int offset,byte[] persistent) {m.WriteBytes(0,Enumerable.Repeat((byte)255,0x800000).ToArray());m.WriteBytes(offset,external);if(persistent.Length!=0)m.WriteBytes(0,persistent);ProgramBytes=Erases=0;}
        public void Save(string path) {
            // Stage beside the destination so a failed write preserves the previous image.
            var destination=Path.GetFullPath(path);
            var temporary=destination+"."+Guid.NewGuid().ToString("N")+".tmp";
            try {
                byte[] image;lock(b.Sync)image=m.ReadBytes(0,0x800000);
                File.WriteAllBytes(temporary,image);
                if(File.Exists(destination))File.Replace(temporary,destination,null);
                else File.Move(temporary,destination);
            } finally {if(File.Exists(temporary))File.Delete(temporary);}
        }
        public void Reset() {Array.Clear(regs,0,regs.Length);wel=powerDown=resetEnable=false;status2=0;opcode=0;cursor=0;busyReads=0;}
        public uint ReadDoubleWord(long o) { lock(b.Sync) { b.Accesses++;if(o==0x20)return 2; if(o==0x50)return ReadData();return regs[o/4];} }
        public void WriteDoubleWord(long o,uint v) {
            lock(b.Sync) {b.Accesses++;regs[o/4]=v;
                if(o==0) regs[0]&=~2u;
                if(o==0x50) WriteData((byte)v);
                if(o==0x110) {opcode=(byte)v;cursor=0; if((regs[0]>>28)==3)return; if((regs[0x100/4]&0x700)==0) Execute();}
                if(o==0x48) {cursor=0;Execute();}
                if(o==0x24) regs[0x20/4]&=~v;
            }
        }
        private void Execute() {
            if(powerDown && opcode!=0xAB && opcode!=0x66 && opcode!=0x99)return;
            switch(opcode) {
            case 6: wel=true; break; case 4: wel=false;break;
            case 0x66:resetEnable=true;break;case 0x99:if(resetEnable) {wel=false;status2=0;powerDown=false;resetEnable=false;}break;
            case 0xAB:powerDown=false;break;case 0xB9:powerDown=true;break;
            case 0x20:Erase(4096);break;case 0x52:Erase(32768);break;case 0xD8:Erase(65536);break;case 0xC7:Erase(0x800000);break;
            case 1:case 0x31:case 5:case 0x35:case 0x9F:case 3:case 0x0B:case 0xEB:case 2:case 0x32:case 0x33:break;
            default:b.Error("Unhandled flash opcode "+opcode.ToString("X2"));break;
            }
        }
        private void Erase(int size) {if(!wel||powerDown)return;int start=(int)(regs[0x48/4]&0x7FFFFF)&~(size-1);m.SetRange(start,size,255);wel=false;busyReads=2;Erases++;}
        private byte ReadData() {
            if(powerDown)return 255;
            int i=cursor++;
            if(opcode==5) {bool busy=busyReads>0;if(busyReads>0)busyReads--;return (byte)((wel?2:0)|(busy?1:0));}
            if(opcode==0x35)return status2;
            if(opcode==0x9F)return new byte[]{0x1F,0x32,0x17}[i%3];
            if(opcode==3||opcode==0xEB||opcode==0x0B)return m.ReadByte((regs[0x48/4]+(uint)i)&0x7FFFFF);
            b.Error("Unhandled flash data read");return 255;
        }
        private void WriteData(byte v) {
            if(!wel||powerDown)return;
            int count=(int)regs[0x40/4]+1;
            if(opcode==0x31) status2=v;
            else if(opcode==1) {if(cursor==1)status2=v;}
            else if(opcode==2||opcode==0x32||opcode==0x33) {
                uint address=(regs[0x48/4]&0x7FFF00)|((regs[0x48/4]+(uint)cursor)&255);
                m.WriteByte(address,(byte)(m.ReadByte(address)&v));ProgramBytes++;
            } else b.Error("Unhandled flash data write");
            if(++cursor>=count) {wel=false;busyReads=2;}
        }
        public byte ReadByte(long o) {lock(b.Sync)return o==0x50?ReadData():(byte)(ReadDoubleWord(o&~3)>>(8*(int)(o&3)));}
        public ushort ReadWord(long o) {lock(b.Sync)return o==0x50?(ushort)(ReadData()|(ReadData()<<8)):(ushort)ReadDoubleWord(o);}
        public void WriteByte(long o,byte v) {lock(b.Sync) {if(o==0x50)WriteData(v);else WriteDoubleWord(o&~3,(ReadDoubleWord(o&~3)&~(255u<<(8*(int)(o&3))))|((uint)v<<(8*(int)(o&3))));}}
        public void WriteWord(long o,ushort v) {lock(b.Sync){if(o==0x50){WriteData((byte)v);WriteData((byte)(v>>8));}else WriteDoubleWord(o,v);}}
        private readonly Board b;private readonly MappedMemory m;private readonly uint[] regs=new uint[256];
        private byte opcode,status2;private bool wel,powerDown,resetEnable;private int cursor,busyReads;
    }

    // Register-level ideal settling for power and analog blocks; no CPU/firmware patching.
    public sealed class Support : IDoubleWordPeripheral,IWordPeripheral,IKnownSize
    {
        public Support(Board board,string kind) {b=board; type=kind;Reset();}
        public long Size {get{return type=="usb"?0x40000:0x400;}}
        public uint ReadDoubleWord(long o) { lock(b.Sync) {b.Accesses++;
            if((type=="adc" || type=="kbdadc") && o==0x40) {
                if(type=="adc")return b.ReadBatteryAdc();
                int[] pins=b.RevisedKeyboard?new[]{9,11}:new[]{2,3};
                return (b.Columns()&(1u<<pins[(conversion++)%2]))!=0?65535u:0u;
            }
            return regs[o/4];} }
        public void WriteDoubleWord(long o,uint v) {lock(b.Sync) {b.Accesses++;uint oldStatus=regs[0];regs[o/4]=v;
            if(type=="usb" && o==0x10)regs[o/4]=0x80000000; // AHB idle, reset/flush requests complete with no attached USB host.
            if(type=="pwr") {if(o==0x18)regs[o/4]|=1<<13;if(o==0x0C)regs[o/4]=(v&~(1u<<26))|((v&(1u<<24))!=0?1u<<26:0);}
            if(type=="syscfg" && o==0x20)regs[o/4]=(v&~256u)|((v&1)!=0?256u:0);
            if(type=="syscfg" && o>=8&&o<24)b.ExtiMux[(o-8)/4]=v;
            if(type=="vref" && o==0)regs[0]=v|((v&1)!=0?8u:0);
            if(type=="adc" || type=="kbdadc") {
                if(o==0)regs[0]=oldStatus&~v;
                if(o==8) {regs[2]&=~(1u<<31);if((v&(1u<<28))!=0)regs[0]|=1<<12;if((v&1)!=0)regs[0]|=1;
                    if((v&2)!=0) {regs[2]&=~3u;regs[0]&=~1u;}
                    if((v&4)!=0) {conversion=0;regs[0]|=12;regs[2]&=~4u;}}
            }
        } }
        public void Reset() {Array.Clear(regs,0,regs.Length);conversion=0;if(type=="syscfg")Array.Clear(b.ExtiMux,0,4);if(type=="usb")regs[4]=0x80000000;if(type=="pwr"){regs[1]=0x6000;regs[6]=0x6000;} if(type=="adc"||type=="kbdadc")regs[2]=1u<<29;}
        public ushort ReadWord(long o) {return (ushort)ReadDoubleWord(o);}
        public void WriteWord(long o,ushort v) {WriteDoubleWord(o,v);}
        private readonly Board b;private readonly string type;private readonly uint[] regs=new uint[65536];
        private int conversion;
    }
}


