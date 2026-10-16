Imports System.Diagnostics
Imports System.IO
Imports System.Management
Imports Microsoft.Win32
Imports System.Threading.Tasks

Public Class WrenchMaster
    Private ReadOnly surfaceColor As Color = Color.FromArgb(60, 70, 73)
    Private ReadOnly accentColor As Color = Color.FromArgb(35, 168, 109)
    Private cleanupCandidates As List(Of String) = New List(Of String)()
    Private previewSucceeded As Boolean
    Private previewTempErrors As Integer
    Private previewLogErrors As Integer

    Private Class CleanupScanResult
        Public ReadOnly Candidates As New List(Of String)()
        Public TempErrors As Integer
        Public LogErrors As Integer
    End Class

    Private Class ActionItem
        Public Property Title As String
        Public Property Description As String
        Public Property Executable As String
        Public Property Arguments As String
        Public Sub New(title As String, description As String, executable As String, Optional arguments As String = "")
            Me.Title = title
            Me.Description = description
            Me.Executable = executable
            Me.Arguments = arguments
        End Sub
    End Class

    Private Function MakeSizeOf(bytes As ULong) As String
        Dim gb As Double = CDbl(bytes) / 1024 / 1024 / 1024
        If gb >= 1 Then Return gb.ToString("0.0") & " GB"
        Return (CDbl(bytes) / 1024 / 1024).ToString("0") & " MB"
    End Function

    Private Sub WrenchMaster_Load(sender As Object, e As EventArgs) Handles MyBase.Load
        AppBitsLabel.Text = If(IntPtr.Size = 8, "(64-bit)", "(32-bit)")
        WinBitsLabel.Text = If(Environment.Is64BitOperatingSystem, "64-bit", "32-bit")
        Me.Text = "Wrenchmaster " & AppBitsLabel.Text
        FormSkin1.Text = Me.Text
        LoadSystemOverview()
        ConfigureCleaner()
        BuildActionPage(TabPage2, "Quick access", "Open familiar Windows tools for routine maintenance.", New ActionItem() {
            New ActionItem("Task Manager", "Review running apps and performance.", "taskmgr.exe"),
            New ActionItem("Startup apps", "Choose which apps start with Windows.", "ms-settings:startupapps"),
            New ActionItem("Storage settings", "See storage use and cleanup recommendations.", "ms-settings:storagesense"),
            New ActionItem("Power settings", "Review sleep and power options.", "ms-settings:powersleep")
        })
        BuildActionPage(TabPage4, "Windows settings", "These shortcuts open settings pages; they do not change settings automatically.", New ActionItem() {
            New ActionItem("Display", "Screen resolution, scale, and display layout.", "ms-settings:display"),
            New ActionItem("Network", "View network status and connection settings.", "ms-settings:network-status"),
            New ActionItem("Sound", "Choose audio devices and adjust sound settings.", "ms-settings:sound"),
            New ActionItem("Notifications", "Review notification preferences.", "ms-settings:notifications")
        })
        BuildActionPage(TabPage3, "Troubleshooting", "Open Windows diagnostic tools and review system events.", New ActionItem() {
            New ActionItem("Event Viewer", "Inspect application and system event logs.", "eventvwr.msc"),
            New ActionItem("Reliability Monitor", "Review a timeline of app and Windows issues.", "perfmon.exe", "/rel"),
            New ActionItem("System Information", "View detailed hardware and Windows information.", "msinfo32.exe"),
            New ActionItem("Windows Update", "Check update status and available updates.", "ms-settings:windowsupdate")
        })
        BuildSystemSummary()
        ConfigureCredits()
        FlatTextBox1.Text = "#23A86D"
        FlatStatusBar1.Text = "Open-source system utility - see Credits"
    End Sub

    Private Sub LoadSystemOverview()
        Dim manufacturer As String = RegistryValue("HKEY_LOCAL_MACHINE\HARDWARE\DESCRIPTION\System\BIOS\", "BaseBoardManufacturer")
        Dim board As String = RegistryValue("HKEY_LOCAL_MACHINE\HARDWARE\DESCRIPTION\System\BIOS\", "BaseBoardProduct")
        Dim processor As String = RegistryValue("HKEY_LOCAL_MACHINE\HARDWARE\DESCRIPTION\System\CentralProcessor\0\", "ProcessorNameString")
        VideoCard.Text = GetFirstWmiValue("SELECT Name FROM Win32_VideoController", "Name")
        WindowsLabel.Text = manufacturer & " " & board & ", " & GetWindowsVersionDescription() & ", " & WinBitsLabel.Text
        InfoLabel.Text = VideoCard.Text & ", " & MakeSizeOf(My.Computer.Info.TotalPhysicalMemory) & " RAM, " & processor
    End Sub

    Private Function GetWindowsVersionDescription() As String
        Return WindowsVersionDetector.ReadVersion().DisplayName
    End Function

    Private Function RegistryValue(key As String, valueName As String) As String
        Try
            Return Convert.ToString(My.Computer.Registry.GetValue(key, valueName, "Unavailable"))
        Catch
            Return "Unavailable"
        End Try
    End Function

    Private Function GetFirstWmiValue(query As String, field As String) As String
        Try
            Using searcher As New ManagementObjectSearcher(query)
                For Each device As ManagementObject In searcher.Get()
                    Return Convert.ToString(device(field))
                Next
            End Using
        Catch
        End Try
        Return "Unavailable"
    End Function

    Private Sub ConfigureCleaner()
        FlatButton7.Text = "Preview"
        FlatButton6.Text = "Open Temp"
        FlatButton5.Text = "Clean selected"
        FlatCheckBox1.Text = "Temporary files (*.tmp)"
        FlatCheckBox1.Checked = False
        FlatCheckBox1.Size = New Size(210, 22)
        FlatCheckBox2.Text = "Log files (*.log)"
        FlatCheckBox2.Checked = True
        FlatCheckBox2.Size = New Size(180, 22)
        AddHandler FlatCheckBox1.CheckedChanged, AddressOf CleanupFilterChanged
        AddHandler FlatCheckBox2.CheckedChanged, AddressOf CleanupFilterChanged
        FlatAlertBox1.Visible = True
        FlatAlertBox2.Visible = True
        SetCleanerStatus(FlatAlertBox1, FlatAlertBox._Kind.Disabled, "Temporary Files")
        SetCleanerStatus(FlatAlertBox2, FlatAlertBox._Kind.Disabled, "Log Files")
        FlatButton5.Enabled = False
        CleanerProgress.Visible = False
        Label1.Visible = True
        Label2.Visible = True
        Label1.Text = "Preview cleanup before deleting"
        Label2.Text = "Only files older than 24 hours in your user Temp folder are included."
    End Sub

    Private Sub CleanupFilterChanged(sender As Object)
        cleanupCandidates.Clear()
        FlatButton5.Enabled = False
        Label1.Text = "Cleanup filters changed"
        Label2.Text = "Run Preview again to refresh the file list."
    End Sub

    Private Sub SetCleanerStatus(statusRow As FlatAlertBox, kind As FlatAlertBox._Kind, message As String)
        statusRow.Kind = kind
        statusRow.Text = message
        statusRow.Invalidate()
    End Sub

    Private Function GetCleanupCandidates(includeTmp As Boolean, includeLog As Boolean) As CleanupScanResult
        Dim result As New CleanupScanResult()
        Dim root As String = Path.GetFullPath(Path.GetTempPath())
        CollectOldTempFiles(root, root, includeTmp, includeLog, result)
        Return result
    End Function

    Private Sub CollectOldTempFiles(root As String, folder As String, includeTmp As Boolean, includeLog As Boolean, result As CleanupScanResult)
        For Each pattern As String In If(includeTmp, New String() {"*.tmp"}, New String() {})
            Try
                For Each filePath As String In Directory.GetFiles(folder, pattern, SearchOption.TopDirectoryOnly)
                    CheckCleanupFile(root, filePath, True, result)
                Next
            Catch
                result.TempErrors += 1
            End Try
        Next
        For Each pattern As String In If(includeLog, New String() {"*.log"}, New String() {})
            Try
                For Each filePath As String In Directory.GetFiles(folder, pattern, SearchOption.TopDirectoryOnly)
                    CheckCleanupFile(root, filePath, False, result)
                Next
            Catch
                result.LogErrors += 1
            End Try
        Next
        Try
            For Each childPath As String In Directory.GetDirectories(folder, "*", SearchOption.TopDirectoryOnly)
                Try
                    If (File.GetAttributes(childPath) And FileAttributes.ReparsePoint) = 0 Then
                        CollectOldTempFiles(root, childPath, includeTmp, includeLog, result)
                    End If
                Catch
                    If includeTmp Then result.TempErrors += 1
                    If includeLog Then result.LogErrors += 1
                End Try
            Next
        Catch
            If includeTmp Then result.TempErrors += 1
            If includeLog Then result.LogErrors += 1
        End Try
    End Sub

    Private Sub CheckCleanupFile(root As String, filePath As String, isTemp As Boolean, result As CleanupScanResult)
        Try
            Dim fullPath As String = Path.GetFullPath(filePath)
            If Not fullPath.StartsWith(root, StringComparison.OrdinalIgnoreCase) Then Return
            If (File.GetAttributes(fullPath) And FileAttributes.ReparsePoint) <> 0 Then Return
            If File.GetLastWriteTimeUtc(fullPath) < DateTime.UtcNow.AddHours(-24) Then result.Candidates.Add(fullPath)
        Catch
            If isTemp Then
                result.TempErrors += 1
            Else
                result.LogErrors += 1
            End If
        End Try
    End Sub

    Private Async Sub FlatButton7_Click(sender As Object, e As EventArgs) Handles FlatButton7.Click
        If Not FlatCheckBox1.Checked AndAlso Not FlatCheckBox2.Checked Then
            MessageBox.Show("Select at least one file type to preview.", "Cleanup preview", MessageBoxButtons.OK, MessageBoxIcon.Information)
            Return
        End If
        cleanupCandidates.Clear()
        previewSucceeded = False
        previewTempErrors = 0
        previewLogErrors = 0
        FlatButton7.Enabled = False
        FlatButton5.Enabled = False
        FlatCheckBox1.Enabled = False
        FlatCheckBox2.Enabled = False
        Label1.Text = "Scanning your Temp folder..."
        Label2.Text = "Locked and inaccessible files are counted as errors."
        Try
            Dim includeTmp As Boolean = FlatCheckBox1.Checked
            Dim includeLog As Boolean = FlatCheckBox2.Checked
            Dim scan As CleanupScanResult = Await Task.Run(Function() GetCleanupCandidates(includeTmp, includeLog))
            cleanupCandidates = scan.Candidates
            previewTempErrors = scan.TempErrors
            previewLogErrors = scan.LogErrors
            previewSucceeded = True
            Dim totalBytes As Long = 0
            For Each filePath As String In cleanupCandidates
                Try
                    totalBytes += New FileInfo(filePath).Length
                Catch
                End Try
            Next
            Label1.Text = cleanupCandidates.Count.ToString("N0") & " eligible files found"
            If previewTempErrors + previewLogErrors > 0 Then
                Label2.Text = FormatBytes(totalBytes) & " found · some folders/files could not be scanned; affected step will show red"
            Else
                Label2.Text = FormatBytes(totalBytes) & " total · files older than 24 hours"
            End If
        Catch ex As Exception
            cleanupCandidates.Clear()
            Label1.Text = "Could not scan the Temp folder"
            Label2.Text = ex.Message
            If FlatCheckBox1.Checked Then SetCleanerStatus(FlatAlertBox1, FlatAlertBox._Kind.Error, "Temporary Files · scan failed")
            If FlatCheckBox2.Checked Then SetCleanerStatus(FlatAlertBox2, FlatAlertBox._Kind.Error, "Log Files · scan failed")
        Finally
            FlatButton7.Enabled = True
            FlatCheckBox1.Enabled = True
            FlatCheckBox2.Enabled = True
            FlatButton5.Enabled = previewSucceeded
        End Try
    End Sub

    Private Async Sub FlatButton5_Click(sender As Object, e As EventArgs) Handles FlatButton5.Click
        If Not previewSucceeded Then
            MessageBox.Show("Run Preview first so the cleaner can show exactly which files are eligible.", "Cleanup", MessageBoxButtons.OK, MessageBoxIcon.Information)
            Return
        End If
        Dim includeTmp As Boolean = FlatCheckBox1.Checked
        Dim includeLog As Boolean = FlatCheckBox2.Checked
        If cleanupCandidates.Count = 0 Then
            If includeTmp Then SetCleanerStatus(FlatAlertBox1, If(previewTempErrors = 0, FlatAlertBox._Kind.Success, FlatAlertBox._Kind.Error), If(previewTempErrors = 0, "Temporary Files · nothing eligible", "Temporary Files · scan incomplete"))
            If includeLog Then SetCleanerStatus(FlatAlertBox2, If(previewLogErrors = 0, FlatAlertBox._Kind.Success, FlatAlertBox._Kind.Error), If(previewLogErrors = 0, "Log Files · nothing eligible", "Log Files · scan incomplete"))
            Label1.Text = If(previewTempErrors + previewLogErrors = 0, "Nothing to clean", "Scan incomplete")
            Label2.Text = "Green means complete; red means some items could not be checked."
            previewSucceeded = False
            FlatButton5.Enabled = False
            Return
        End If
        If MessageBox.Show("Delete " & cleanupCandidates.Count.ToString("N0") & " eligible old temporary files? Files currently in use will be skipped.", "Confirm cleanup", MessageBoxButtons.YesNo, MessageBoxIcon.Warning) <> DialogResult.Yes Then Return

        Dim filesToDelete As New List(Of String)(cleanupCandidates)
        If includeTmp Then SetCleanerStatus(FlatAlertBox1, FlatAlertBox._Kind.Disabled, "Temporary Files · waiting")
        If includeLog Then SetCleanerStatus(FlatAlertBox2, FlatAlertBox._Kind.Disabled, "Log Files · waiting")
        FlatButton5.Enabled = False
        FlatButton7.Enabled = False
        FlatCheckBox1.Enabled = False
        FlatCheckBox2.Enabled = False
        CleanerProgress.Visible = True
        CleanerProgress.Value = 0
        Dim deletedTemp As Integer = 0
        Dim deletedLogs As Integer = 0
        Dim failedTemp As Integer = previewTempErrors
        Dim failedLogs As Integer = previewLogErrors
        Dim total As Integer = filesToDelete.Count
        Try
            For index As Integer = 0 To total - 1
                Dim filePath As String = filesToDelete(index)
                Dim isTemp As Boolean = Path.GetExtension(filePath).Equals(".tmp", StringComparison.OrdinalIgnoreCase)
                Try
                    If File.Exists(filePath) Then File.Delete(filePath)
                    If File.Exists(filePath) Then
                        If isTemp Then failedTemp += 1 Else failedLogs += 1
                    ElseIf isTemp Then
                        deletedTemp += 1
                    Else
                        deletedLogs += 1
                    End If
                Catch
                    If isTemp Then failedTemp += 1 Else failedLogs += 1
                End Try
                If index Mod 10 = 0 OrElse index = total - 1 Then
                    CleanerProgress.Value = CInt(Math.Min(100, ((index + 1) * 100.0) / total))
                    Label1.Text = "Cleaning selected temporary files"
                    Label2.Text = (index + 1).ToString("N0") & " of " & total.ToString("N0") & " checked"
                    Await Task.Yield()
                End If
            Next
            If includeTmp Then SetCleanerStatus(FlatAlertBox1, If(failedTemp = 0, FlatAlertBox._Kind.Success, FlatAlertBox._Kind.Error), If(failedTemp = 0, "Temporary Files · " & deletedTemp.ToString("N0") & " removed", "Temporary Files · " & failedTemp.ToString("N0") & " could not be cleaned"))
            If includeLog Then SetCleanerStatus(FlatAlertBox2, If(failedLogs = 0, FlatAlertBox._Kind.Success, FlatAlertBox._Kind.Error), If(failedLogs = 0, "Log Files · " & deletedLogs.ToString("N0") & " removed", "Log Files · " & failedLogs.ToString("N0") & " could not be cleaned"))
            Label1.Text = (deletedTemp + deletedLogs).ToString("N0") & " files removed"
            Label2.Text = If(failedTemp + failedLogs = 0, "Cleanup completed successfully.", (failedTemp + failedLogs).ToString("N0") & " items could not be scanned or removed; see red status rows.")
        Finally
            cleanupCandidates.Clear()
            previewSucceeded = False
            CleanerProgress.Visible = False
            FlatButton5.Enabled = False
            FlatButton7.Enabled = True
            FlatCheckBox1.Enabled = True
            FlatCheckBox2.Enabled = True
        End Try
    End Sub
    Private Sub FlatButton6_Click(sender As Object, e As EventArgs) Handles FlatButton6.Click
        Try
            Process.Start(New ProcessStartInfo("explorer.exe", """" & Path.GetTempPath() & """") With {.UseShellExecute = True})
        Catch ex As Exception
            MessageBox.Show("Could not open the Temp folder: " & ex.Message, "Open Temp", MessageBoxButtons.OK, MessageBoxIcon.Error)
        End Try
    End Sub

    Private Function FormatBytes(bytes As Long) As String
        If bytes >= 1024L * 1024 * 1024 Then Return (bytes / 1024.0 / 1024 / 1024).ToString("0.0") & " GB"
        If bytes >= 1024L * 1024 Then Return (bytes / 1024.0 / 1024).ToString("0.0") & " MB"
        Return (bytes / 1024.0).ToString("0") & " KB"
    End Function

    Private Sub BuildActionPage(page As TabPage, heading As String, description As String, items As ActionItem())
        page.Controls.Clear()
        page.BackColor = surfaceColor
        Dim title As New Label With {.Text = heading, .Font = New Font("Segoe UI", 18.0!, FontStyle.Bold), .ForeColor = Color.White, .BackColor = Color.Transparent, .Location = New Point(24, 20), .AutoSize = True}
        Dim subtitle As New Label With {.Text = description, .Font = New Font("Segoe UI", 10.0!), .ForeColor = Color.Gainsboro, .BackColor = Color.Transparent, .Location = New Point(26, 58), .Size = New Size(870, 28)}
        page.Controls.Add(title)
        page.Controls.Add(subtitle)
        For index As Integer = 0 To items.Length - 1
            Dim col As Integer = index Mod 2
            Dim row As Integer = index \ 2
            Dim card As New Panel With {.BackColor = Color.FromArgb(48, 55, 58), .Location = New Point(24 + col * 445, 102 + row * 154), .Size = New Size(420, 136), .BorderStyle = BorderStyle.FixedSingle}
            Dim icon As New Label With {.Text = items(index).Title.Substring(0, 1).ToUpperInvariant(), .Font = New Font("Segoe UI", 16.0!, FontStyle.Bold), .ForeColor = Color.White, .BackColor = accentColor, .TextAlign = ContentAlignment.MiddleCenter, .Location = New Point(15, 20), .Size = New Size(42, 42)}
            Dim name As New Label With {.Text = items(index).Title, .Font = New Font("Segoe UI", 12.0!, FontStyle.Bold), .ForeColor = Color.White, .BackColor = Color.Transparent, .Location = New Point(70, 18), .AutoSize = True}
            Dim detail As New Label With {.Text = items(index).Description, .Font = New Font("Segoe UI", 9.0!), .ForeColor = Color.Gainsboro, .BackColor = Color.Transparent, .Location = New Point(70, 45), .Size = New Size(330, 38)}
            Dim launch As New Button With {.Text = "Open", .Font = New Font("Segoe UI", 9.0!), .Location = New Point(70, 88), .Size = New Size(90, 30), .FlatStyle = FlatStyle.Flat, .BackColor = accentColor, .ForeColor = Color.White, .Tag = items(index)}
            AddHandler launch.Click, AddressOf LaunchAction_Click
            card.Controls.Add(icon)
            card.Controls.Add(name)
            card.Controls.Add(detail)
            card.Controls.Add(launch)
            page.Controls.Add(card)
        Next
    End Sub

    Private Sub LaunchAction_Click(sender As Object, e As EventArgs)
        Dim item As ActionItem = DirectCast(DirectCast(sender, Control).Tag, ActionItem)
        Try
            Process.Start(New ProcessStartInfo(item.Executable, item.Arguments) With {.UseShellExecute = True})
        Catch ex As Exception
            MessageBox.Show("Could not open " & item.Title & ": " & ex.Message, "Windows tool", MessageBoxButtons.OK, MessageBoxIcon.Error)
        End Try
    End Sub

    Private Sub BuildSystemSummary()
        TabPage5.Controls.Clear()
        TabPage5.BackColor = surfaceColor
        Dim heading As New Label With {.Text = "System summary", .Font = New Font("Segoe UI", 18.0!, FontStyle.Bold), .ForeColor = Color.White, .BackColor = Color.Transparent, .Location = New Point(24, 20), .AutoSize = True}
        Dim note As New Label With {.Text = "Read-only overview. No account names or unique device identifiers are collected.", .Font = New Font("Segoe UI", 10.0!), .ForeColor = Color.Gainsboro, .BackColor = Color.Transparent, .Location = New Point(26, 58), .Size = New Size(850, 28)}
        TabPage5.Controls.Add(heading)
        TabPage5.Controls.Add(note)
        AddSummaryRow("Windows", GetWindowsVersionDescription() & " · " & WinBitsLabel.Text, 112)
        AddSummaryRow("Processor", RegistryValue("HKEY_LOCAL_MACHINE\HARDWARE\DESCRIPTION\System\CentralProcessor\0\", "ProcessorNameString"), 164)
        AddSummaryRow("Memory", MakeSizeOf(My.Computer.Info.TotalPhysicalMemory), 216)
        AddSummaryRow("Graphics", VideoCard.Text, 268)
        AddSummaryRow("Mainboard", WindowsLabel.Text.Split(","c)(0), 320)
        Try
            Dim drive As New DriveInfo(Path.GetPathRoot(Environment.SystemDirectory))
            AddSummaryRow("System drive", drive.Name & " · " & FormatBytes(drive.AvailableFreeSpace) & " free of " & FormatBytes(drive.TotalSize), 372)
        Catch
            AddSummaryRow("System drive", "Storage information unavailable", 372)
        End Try
    End Sub

    Private Sub AddSummaryRow(caption As String, value As String, top As Integer)
        Dim row As New Panel With {.BackColor = Color.FromArgb(48, 55, 58), .Location = New Point(24, top), .Size = New Size(870, 44)}
        row.Controls.Add(New Label With {.Text = caption, .Font = New Font("Segoe UI", 9.0!, FontStyle.Bold), .ForeColor = Color.LightGray, .BackColor = Color.Transparent, .Location = New Point(14, 11), .Size = New Size(140, 22)})
        row.Controls.Add(New Label With {.Text = value, .Font = New Font("Segoe UI", 9.0!), .ForeColor = Color.White, .BackColor = Color.Transparent, .Location = New Point(160, 11), .Size = New Size(695, 22), .AutoEllipsis = True})
        TabPage5.Controls.Add(row)
    End Sub

    Private Sub ConfigureCredits()
        TabPage7.Text = "Credits"
        TabPage7.Controls.Clear()
        TabPage7.Controls.Add(New Label With {.AutoSize = True, .BackColor = Color.Transparent, .ForeColor = Color.White, .Font = New Font("Segoe UI", 11.0!), .Location = New Point(24, 24), .Text = "Wrenchmaster (2017)" & Environment.NewLine & "Original application: Mauro Leoci" & Environment.NewLine & "Flat UI theme: iSynthesis; DrawArrow: AeonHack" & Environment.NewLine & "MD5 example: Stack Overflow community"})
    End Sub

    Private Sub FlatButton3_Click(sender As Object, e As EventArgs) Handles FlatButton3.Click
        Dim colorText As String = FlatTextBox1.Text.Trim()
        Try
            Dim selectedColor As Color = ColorTranslator.FromHtml(colorText)
            If selectedColor.IsEmpty Then Throw New FormatException("Enter a named color or a color such as #23A86D.")
            FormSkin1.HeaderColor = selectedColor
            FlatTabControl1.BaseColor = selectedColor
            FlatTabControl1.ActiveColor = selectedColor
            FormSkin1.Refresh()
            FlatTabControl1.Refresh()
            MessageBox.Show("Accent color applied.", "Appearance", MessageBoxButtons.OK, MessageBoxIcon.Information)
        Catch ex As Exception
            MessageBox.Show("Enter a valid color, for example #23A86D.", "Appearance", MessageBoxButtons.OK, MessageBoxIcon.Information)
        End Try
    End Sub

    Private Sub FlatButton4_Click_1(sender As Object, e As EventArgs) Handles FlatButton4.Click
        RegEditBackup.Show()
    End Sub

    Private Sub FlatStickyButton1_Click(sender As Object, e As EventArgs) Handles FlatStickyButton1.Click
        Me.Close()
    End Sub

    Private Sub FlatStickyButton2_Click(sender As Object, e As EventArgs) Handles FlatStickyButton2.Click
        Me.WindowState = FormWindowState.Minimized
    End Sub
End Class








