// CPU-only, local helper for Open Dock. No network, tuning or installer APIs.
using System;
using System.Collections.Generic;
using System.IO;
using System.IO.Pipes;
using System.Security.AccessControl;
using System.Security.Principal;
using System.ServiceProcess;
using System.Text;
using System.Threading;
using System.Web.Script.Serialization;
using LibreHardwareMonitor.Hardware;
using LibreHardwareMonitor.PawnIo;

namespace OpenDock.Sensors
{
    internal static class Program
    {
        internal const string ServiceName = "OpenDockCpuSensor";
        internal const string PipeName = "OpenDock.CpuSensor";

        private static readonly object HardwareLock = new object();
        private static Computer computer;
        private static readonly JavaScriptSerializer Serializer = new JavaScriptSerializer();

        internal static void CloseComputer()
        {
            lock (HardwareLock)
            {
                if (computer != null)
                {
                    try { computer.Close(); } catch { }
                    computer = null;
                }
            }
        }

        private static Dictionary<string, object> Sample(long seq)
        {
            lock (HardwareLock)
            {
                var result = new Dictionary<string, object>
                {
                    { "schema", 1 },
                    { "sequence", seq },
                    { "at", DateTimeOffset.UtcNow.ToUnixTimeMilliseconds() },
                    { "celsius", null },
                    { "source", "lhm-direct-package" },
                    { "status", "sensor-unavailable" }
                };
                if (!PawnIo.IsInstalled || PawnIo.Version < new Version(2, 2))
                {
                    result["status"] = "driver-required";
                    return result;
                }
                using (var identity = WindowsIdentity.GetCurrent())
                {
                    if (!new WindowsPrincipal(identity).IsInRole(WindowsBuiltInRole.Administrator))
                    {
                        result["status"] = "admin-required";
                        return result;
                    }
                }
                if (computer == null)
                {
                    computer = new Computer
                    {
                        IsCpuEnabled = true,
                        IsGpuEnabled = false,
                        IsMemoryEnabled = false,
                        IsMotherboardEnabled = false,
                        IsControllerEnabled = false,
                        IsNetworkEnabled = false,
                        IsStorageEnabled = false,
                        IsBatteryEnabled = false,
                        IsPowerMonitorEnabled = false
                    };
                    computer.Open();
                }
                var candidates = new List<SensorCandidate>();
                foreach (IHardware hardware in computer.Hardware)
                {
                    if (hardware.HardwareType != HardwareType.Cpu) continue;
                    hardware.Update();
                    foreach (ISensor sensor in hardware.Sensors)
                    {
                        if (sensor.SensorType == SensorType.Temperature && sensor.Value.HasValue)
                        {
                            candidates.Add(new SensorCandidate
                            {
                                IsCpu = true,
                                HardwareId = hardware.Identifier.ToString(),
                                HardwareName = hardware.Name,
                                SensorId = sensor.Identifier.ToString(),
                                SensorName = sensor.Name,
                                Celsius = sensor.Value.Value
                            });
                        }
                    }
                }
                var reading = TemperatureSelection.Select(candidates);
                if (reading != null)
                {
                    result["status"] = "ready";
                    result["celsius"] = reading.Celsius;
                    result["hardware"] = reading.HardwareName;
                    result["sensor"] = reading.SensorName;
                    result["sensorId"] = reading.SensorId;
                }
                result["at"] = DateTimeOffset.UtcNow.ToUnixTimeMilliseconds();
                return result;
            }
        }

        internal static string SampleJson(long seq, out bool fatalError)
        {
            fatalError = false;
            try
            {
                return Serializer.Serialize(Sample(seq));
            }
            catch (Exception error)
            {
                fatalError = true;
                try
                {
                    Console.Error.WriteLine("[cpu-sensor] " + error.GetType().Name + ": " + error.Message);
                }
                catch { }
                CloseComputer();
                return Serializer.Serialize(new
                {
                    schema = 1,
                    sequence = seq,
                    at = DateTimeOffset.UtcNow.ToUnixTimeMilliseconds(),
                    celsius = (float?)null,
                    source = "lhm-direct-package",
                    status = "sensor-error"
                });
            }
        }

        public static int Main(string[] args)
        {
            if (args != null && args.Length == 1 && args[0] == "--service")
            {
                ServiceBase.Run(new CpuSensorService());
                return 0;
            }
            if (args != null && args.Length > 0)
            {
                return 2;
            }

            // Wait before opening hardware: the native parent attaches its job
            // object before sending the first command. EOF means parent closed.
            long sequence = 0;
            try
            {
                string command;
                while ((command = Console.ReadLine()) != null)
                {
                    if (command != "sample") return 2;
                    sequence++;
                    bool fatalError;
                    Console.WriteLine(SampleJson(sequence, out fatalError));
                    if (fatalError) return 1;
                }
                return 0;
            }
            finally
            {
                CloseComputer();
            }
        }
    }

    internal sealed class CpuSensorService : ServiceBase
    {
        private readonly ManualResetEvent stopEvent = new ManualResetEvent(false);
        private readonly object pipeLock = new object();
        private NamedPipeServerStream activePipe;
        private Thread serverThread;

        public CpuSensorService()
        {
            ServiceName = Program.ServiceName;
            CanStop = true;
            CanShutdown = true;
            AutoLog = false;
        }

        protected override void OnStart(string[] args)
        {
            stopEvent.Reset();
            serverThread = new Thread(RunPipeLoop);
            serverThread.IsBackground = true;
            serverThread.Start();
        }

        protected override void OnStop()
        {
            StopServer();
        }

        protected override void OnShutdown()
        {
            StopServer();
        }

        private void StopServer()
        {
            stopEvent.Set();
            lock (pipeLock)
            {
                if (activePipe != null)
                {
                    try { activePipe.Dispose(); } catch { }
                    activePipe = null;
                }
            }
            if (serverThread != null && serverThread.IsAlive)
            {
                serverThread.Join(3000);
            }
            Program.CloseComputer();
        }

        private static PipeSecurity BuildPipeSecurity()
        {
            var security = new PipeSecurity();
            security.AddAccessRule(new PipeAccessRule(
                new SecurityIdentifier(WellKnownSidType.NetworkSid, null),
                PipeAccessRights.FullControl,
                AccessControlType.Deny));
            security.AddAccessRule(new PipeAccessRule(
                new SecurityIdentifier(WellKnownSidType.AnonymousSid, null),
                PipeAccessRights.FullControl,
                AccessControlType.Deny));
            security.AddAccessRule(new PipeAccessRule(
                new SecurityIdentifier(WellKnownSidType.LocalSystemSid, null),
                PipeAccessRights.FullControl,
                AccessControlType.Allow));
            security.AddAccessRule(new PipeAccessRule(
                new SecurityIdentifier(WellKnownSidType.BuiltinAdministratorsSid, null),
                PipeAccessRights.FullControl,
                AccessControlType.Allow));
            security.AddAccessRule(new PipeAccessRule(
                new SecurityIdentifier(WellKnownSidType.AuthenticatedUserSid, null),
                PipeAccessRights.ReadWrite,
                AccessControlType.Allow));
            return security;
        }

        private void RunPipeLoop()
        {
            PipeSecurity security = BuildPipeSecurity();
            while (!stopEvent.WaitOne(0))
            {
                NamedPipeServerStream pipe = null;
                try
                {
                    pipe = new NamedPipeServerStream(
                        Program.PipeName,
                        PipeDirection.InOut,
                        2,
                        PipeTransmissionMode.Byte,
                        PipeOptions.Asynchronous,
                        256,
                        4096,
                        security);

                    lock (pipeLock)
                    {
                        if (stopEvent.WaitOne(0))
                        {
                            pipe.Dispose();
                            return;
                        }
                        activePipe = pipe;
                    }

                    IAsyncResult ar = pipe.BeginWaitForConnection(null, null);
                    int idx = WaitHandle.WaitAny(new WaitHandle[] { ar.AsyncWaitHandle, stopEvent });
                    if (idx == 1)
                    {
                        return;
                    }
                    pipe.EndWaitForConnection(ar);
                    HandleClient(pipe);
                }
                catch (ObjectDisposedException)
                {
                    if (stopEvent.WaitOne(0)) return;
                }
                catch (IOException)
                {
                    if (stopEvent.WaitOne(0)) return;
                }
                catch (Exception)
                {
                    if (stopEvent.WaitOne(0)) return;
                    stopEvent.WaitOne(250);
                }
                finally
                {
                    lock (pipeLock)
                    {
                        if (ReferenceEquals(activePipe, pipe))
                        {
                            activePipe = null;
                        }
                    }
                    if (pipe != null)
                    {
                        try { pipe.Dispose(); } catch { }
                    }
                    Program.CloseComputer();
                }
            }
        }

        private void HandleClient(NamedPipeServerStream pipe)
        {
            long sequence = 0;
            var utf8 = new UTF8Encoding(false);
            using (var writer = new StreamWriter(pipe, utf8, 4096, true))
            {
                writer.AutoFlush = true;
                writer.NewLine = "\n";
                while (!stopEvent.WaitOne(0) && pipe.IsConnected)
                {
                    string command = ReadBoundedLine(pipe, 32);
                    if (command == null || command != "sample")
                    {
                        return;
                    }
                    sequence++;
                    bool fatalError;
                    writer.WriteLine(Program.SampleJson(sequence, out fatalError));
                    if (fatalError)
                    {
                        return;
                    }
                }
            }
        }

        private static string ReadBoundedLine(Stream stream, int maxBytes)
        {
            var buffer = new byte[maxBytes];
            int count = 0;
            while (true)
            {
                int b = stream.ReadByte();
                if (b < 0)
                {
                    return null;
                }
                if (b == '\n')
                {
                    return Encoding.ASCII.GetString(buffer, 0, count);
                }
                if (b == '\r')
                {
                    continue;
                }
                if (count >= maxBytes)
                {
                    return null;
                }
                buffer[count++] = (byte)b;
            }
        }
    }
}
