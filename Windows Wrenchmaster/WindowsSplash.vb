Imports Microsoft.Win32
Imports System.Drawing
Imports System.Windows.Forms

Public Class WindowsSplash
    Inherits Form

    Private ReadOnly skin As FormSkin
    Private ReadOnly logo As PictureBox
    Private ReadOnly heading As Label
    Private ReadOnly versionInfo As Label
    Private ReadOnly startButton As FlatButton
    Private ReadOnly launchTimer As Timer

    Public Sub New()
        Me.FormBorderStyle = FormBorderStyle.None
        Me.StartPosition = FormStartPosition.CenterScreen
        Me.ClientSize = New Size(661, 463)
        Me.BackColor = Color.FromArgb(45, 47, 49)
        Me.TransparencyKey = Color.Fuchsia
        Me.ShowInTaskbar = True
        Me.Text = "Wrenchmaster"

        skin = New FormSkin()
        skin.Dock = DockStyle.Fill
        skin.BaseColor = Color.FromArgb(45, 47, 49)
        skin.BorderColor = Color.FromArgb(33, 38, 39)
        skin.FlatColor = Color.FromArgb(35, 168, 109)
        skin.HeaderColor = Color.FromArgb(33, 38, 39)
        skin.HeaderMaximize = False
        skin.Text = "Wrenchmaster"
        Me.Controls.Add(skin)

        heading = New Label()
        heading.AutoSize = False
        heading.TextAlign = ContentAlignment.MiddleCenter
        heading.ForeColor = Color.White
        heading.BackColor = Color.Transparent
        heading.Font = New Font("Segoe UI", 16.0!, FontStyle.Regular)
        heading.Location = New Point(30, 55)
        heading.Size = New Size(601, 34)
        heading.Text = "Wrenchmaster"
        skin.Controls.Add(heading)

        logo = New PictureBox()
        logo.BackColor = Color.FromArgb(29, 29, 29)
        logo.Location = New Point(232, 125)
        logo.Size = New Size(196, 105)
        logo.SizeMode = PictureBoxSizeMode.Zoom
        logo.TabStop = False
        skin.Controls.Add(logo)

        versionInfo = New Label()
        versionInfo.AutoSize = False
        versionInfo.TextAlign = ContentAlignment.MiddleCenter
        versionInfo.ForeColor = Color.FromArgb(190, 198, 208)
        versionInfo.BackColor = Color.Transparent
        versionInfo.Font = New Font("Segoe UI", 10.0!)
        versionInfo.Location = New Point(50, 255)
        versionInfo.Size = New Size(561, 30)
        skin.Controls.Add(versionInfo)

        startButton = New FlatButton()
        startButton.BackColor = Color.Transparent
        startButton.BaseColor = Color.FromArgb(35, 168, 109)
        startButton.Cursor = Cursors.Hand
        startButton.Font = New Font("Segoe UI", 11.0!)
        startButton.Location = New Point(232, 335)
        startButton.Name = "StartWrenchmasterButton"
        startButton.Rounded = False
        startButton.Size = New Size(196, 41)
        startButton.Text = "Continue"
        startButton.TextColor = Color.White
        AddHandler startButton.Click, AddressOf StartMainForm
        skin.Controls.Add(startButton)

        launchTimer = New Timer()
        launchTimer.Interval = 1800
        AddHandler launchTimer.Tick, AddressOf LaunchTimer_Tick
        AddHandler Me.Shown, AddressOf WindowsSplash_Shown
    End Sub

    Private Sub WindowsSplash_Shown(sender As Object, e As EventArgs)
        DetectWindowsAndSetLogo()
        launchTimer.Start()
    End Sub

    Private Sub DetectWindowsAndSetLogo()
        Dim windowsInfo As WindowsVersionInfo = WindowsVersionDetector.ReadVersion()
        versionInfo.Text = windowsInfo.DisplayName

        If windowsInfo.IsWindows11 Then
            logo.Image = My.Resources.Windows11
        ElseIf windowsInfo.ProductName.IndexOf("Windows 10", StringComparison.OrdinalIgnoreCase) >= 0 Then
            logo.Image = My.Resources.Windows10
        ElseIf windowsInfo.ProductName.IndexOf("Windows 8.1", StringComparison.OrdinalIgnoreCase) >= 0 Then
            logo.Image = My.Resources.Windows81
        ElseIf windowsInfo.ProductName.IndexOf("Windows 8", StringComparison.OrdinalIgnoreCase) >= 0 Then
            logo.Image = My.Resources.Windows8
        ElseIf windowsInfo.ProductName.IndexOf("Windows 7", StringComparison.OrdinalIgnoreCase) >= 0 Then
            logo.Image = My.Resources.Windows7
        ElseIf windowsInfo.ProductName.IndexOf("Windows Vista", StringComparison.OrdinalIgnoreCase) >= 0 Then
            logo.Image = My.Resources.WindowsVista
        ElseIf windowsInfo.ProductName.IndexOf("Windows XP", StringComparison.OrdinalIgnoreCase) >= 0 Then
            logo.Image = My.Resources.WindowsXP
        ElseIf windowsInfo.ProductName.IndexOf("Windows 2000", StringComparison.OrdinalIgnoreCase) >= 0 Then
            logo.Image = My.Resources.Windows2000
        ElseIf windowsInfo.ProductName.IndexOf("Windows ME", StringComparison.OrdinalIgnoreCase) >= 0 Then
            logo.Image = My.Resources.WindowsME
        ElseIf windowsInfo.ProductName.IndexOf("Windows 98", StringComparison.OrdinalIgnoreCase) >= 0 Then
            logo.Image = My.Resources.Windows98
        ElseIf windowsInfo.ProductName.IndexOf("Windows 95", StringComparison.OrdinalIgnoreCase) >= 0 Then
            logo.Image = My.Resources.Windows95
        Else
            logo.Image = My.Resources.Windows
        End If
    End Sub
    Private Sub LaunchTimer_Tick(sender As Object, e As EventArgs)
        launchTimer.Stop()
        StartMainForm(sender, e)
    End Sub

    Private Sub StartMainForm(sender As Object, e As EventArgs)
        launchTimer.Stop()
        startButton.Enabled = False
        Dim main As New WrenchMaster()
        AddHandler main.FormClosed, AddressOf MainForm_Closed
        main.Show()
        Me.Hide()
    End Sub

    Private Sub MainForm_Closed(sender As Object, e As FormClosedEventArgs)
        Me.Close()
    End Sub
End Class

