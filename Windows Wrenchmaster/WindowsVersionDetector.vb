Imports Microsoft.Win32
Imports System.Management

Friend NotInheritable Class WindowsVersionInfo
    Public Property ProductName As String = "Windows"
    Public Property DisplayVersion As String = ""
    Public Property BuildNumber As Integer
    Public Property IsClient As Boolean

    Public ReadOnly Property IsWindows11 As Boolean
        Get
            Return IsClient AndAlso BuildNumber >= 22000
        End Get
    End Property

    Public ReadOnly Property DisplayName As String
        Get
            Dim name As String = ProductName
            If IsWindows11 AndAlso name.IndexOf("Windows 10", StringComparison.OrdinalIgnoreCase) >= 0 Then
                name = name.Replace("Windows 10", "Windows 11")
            ElseIf IsWindows11 AndAlso name.Equals("Windows", StringComparison.OrdinalIgnoreCase) Then
                name = "Windows 11"
            End If
            If DisplayVersion.Length > 0 Then name &= " " & DisplayVersion
            If BuildNumber > 0 Then name &= " (build " & BuildNumber.ToString() & ")"
            Return name
        End Get
    End Property
End Class

Friend NotInheritable Class WindowsVersionDetector
    Private Sub New()
    End Sub

    Public Shared Function ReadVersion() As WindowsVersionInfo
        Dim info As New WindowsVersionInfo()
        Try
            Using searcher As New ManagementObjectSearcher("SELECT Caption, BuildNumber, ProductType FROM Win32_OperatingSystem")
                For Each os As ManagementObject In searcher.Get()
                    info.ProductName = Convert.ToString(os("Caption"))
                    Integer.TryParse(Convert.ToString(os("BuildNumber")), info.BuildNumber)
                    info.IsClient = Convert.ToUInt32(os("ProductType")) = 1UI
                    Exit For
                Next
            End Using
        Catch
            ReadFromRegistry(info)
        End Try
        If info.BuildNumber = 0 OrElse info.ProductName = "Windows" Then ReadFromRegistry(info)
        Return info
    End Function

    Private Shared Sub ReadFromRegistry(info As WindowsVersionInfo)
        Try
            Using baseKey As RegistryKey = RegistryKey.OpenBaseKey(RegistryHive.LocalMachine, RegistryView.Registry64)
                Using key As RegistryKey = baseKey.OpenSubKey("SOFTWARE\Microsoft\Windows NT\CurrentVersion")
                    If key IsNot Nothing Then
                        info.ProductName = Convert.ToString(key.GetValue("ProductName", info.ProductName))
                        Dim buildText As String = Convert.ToString(key.GetValue("CurrentBuildNumber", key.GetValue("CurrentBuild", "0")))
                        Integer.TryParse(buildText, info.BuildNumber)
                        info.DisplayVersion = Convert.ToString(key.GetValue("DisplayVersion", ""))
                        Dim installType As String = Convert.ToString(key.GetValue("InstallationType", ""))
                        info.IsClient = installType.Equals("Client", StringComparison.OrdinalIgnoreCase)
                        If Not info.IsClient AndAlso installType.Length = 0 Then
                            info.IsClient = info.ProductName.IndexOf("Windows 10", StringComparison.OrdinalIgnoreCase) >= 0 OrElse info.ProductName.IndexOf("Windows 11", StringComparison.OrdinalIgnoreCase) >= 0
                        End If
                    End If
                End Using
            End Using
        Catch
        End Try
    End Sub
End Class
